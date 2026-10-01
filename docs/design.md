# LavaTUI — Layout & Visual Design Spec

Status: **contract** for `lava-xxx` (TUI shell), `lava-bdj`/`lava-y7g`
(styles), `lava-ef7` (faces), `lava-h0f` (palettes/perf) and `lava-5ak`
(lighting). If the code and this doc disagree, fix one of them on purpose,
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
| **content area** | Terminal minus outer margins, minus the status bar row when it's shown. |
| **lamp region** | The part of the content area that belongs to the lamp. |
| **glass** | Lamp frame mode: a lava-lamp silhouette (cap, bottle, base) drawn inside the lamp region. |
| **bleed** | Lamp frame mode: the fluid fills the lamp region edge to edge, with no silhouette. |
| **panel** | The clock + pomodoro block, placed beside the lamp (right panel) or below it (bottom panel). |
| **chip** | The single-line fallback for the panel: ` 14:32 ` or ` ▸ 18:24 `, drawn over a corner of the lamp. |
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
| **Micro** | < 20 cols or < 8 rows | bleed, whole screen | – | – | – | – |
| **Tiny** | 20–39 × 8–13 | bleed, whole screen | chip `14:32` | chip `▸ 18:24` (replaces clock while running) | – | – |
| **Small** | 40–79 × 14–23 | bleed | chip, or right panel with M face if ≥ 60 % width remains | chip or panel: time + bar | yes: style · palette | `? help` and as many more as fit |
| **Medium** | 80–119 × 24–35 | **glass** (auto) | panel, M face | panel: label, time, bar, dots | yes | most hints |
| **Large** | 120–199 × 36–55 | glass, bigger margins | panel, L face + date line | full | yes | all hints |
| **Huge** | ≥ 200 × ≥ 56 | glass, proportional margins | panel, XL face + date | full | yes | all hints |

At any size, a portrait shape (narrow and tall) moves the panel *below*
the lamp (§1.4).

### 1.3 Element rules (authoritative)

| Element | Shown when | Variant rules |
|---|---|---|
| **Lamp** | always (if `cols < 4` or `rows < 2`, the screen is painted `bg`, nothing else) | frame per §2.1 |
| **Status bar** | `rows ≥ 14 && cols ≥ 30 && status_bar_on` and not minimal mode | segments drop per §4.1 |
| **Panel** | the placement algorithm (§1.4) finds a slot | face variant = largest that fits the panel's inner rect |
| **Chip** | no panel, clock or pomodoro enabled, `cols ≥ 20 && rows ≥ 8` | shows pomodoro while one is running, else the clock |
| **Date line** | in panel, `rows ≥ 36`, and the panel still fits | `thu 1 oct`, dim, lowercase |
| **Pomodoro label** `focus` / `break` | panel inner width ≥ 18 | — |
| **Cycle dots** `●●○○` | panel inner width ≥ 22 | right-aligned on the label line |
| **Toasts** | `cols ≥ 16 && rows ≥ 4` | truncated by dropping the suffix (`braille 6/12` → `braille`), never mid-word |
| **Outer margins** | glass mode only (bleed is edge-to-edge) | vertical (top and above status bar): 0 if rows < 24, 1 if < 36, 2 if < 56, else `round(rows × 0.04)`; horizontal: 2 if cols < 120, 4 if < 200, else `round(cols × 0.03)` |

**Hide priority.** When space runs out, things go in this order (first to
go at the top). The lamp is never hidden.

1. Extended key hints (dropped right-to-left until only `? help` is left, then that too)
2. Date line
3. Cycle dots, then the pomodoro phase label
4. Outer margins (shrink to 0)
5. Clock face size (XL → L → M → S → `text`)
6. Glass silhouette (→ bleed)
7. Panel (→ collapses into the chip; nothing is lost but size)
8. Status bar
9. Clock chip (a running pomodoro chip outranks it)
10. Pomodoro chip
11. ~~Lamp~~ — never

### 1.4 Panel placement algorithm

```
panel_w   = clamp(round(cols × 0.30), 22, 36)      // incl. 1-col inner padding each side
gutter    = clamp(cols / 16, 4, 12)                // glass mode: lamp ↔ panel
panel_h   = face_h + (date? 2) + 2 + 3             // face, gap, label/time/bar

GLASS mode (lamp height Ht, width W = round(Ht × 0.8)):
  1. Right panel: Ht = min(content_rows, (content_cols − gutter − panel_w) / 0.8).
     Accept if Ht ≥ 20 and Ht ≥ 0.85 × content_rows.
     Lamp + gutter + panel form ONE group, centred horizontally; the panel
     is vertically centred on the lamp.
  2. Bottom panel: Ht = min(content_rows − panel_h − 2, (content_cols − 4) / 0.8).
     Accept if Ht ≥ 20. The panel block is centred under the base.
  3. Otherwise no panel → chip (and re-evaluate frame: glass may still fit).

BLEED mode:
  1. A ≥ 1.0 → right panel if lamp keeps ≥ 60 % of cols and ≥ 24 cols.
  2. A < 1.0 → bottom panel if lamp keeps ≥ 60 % of rows and ≥ 10 rows.
  3. Otherwise chip.
```

Centring: whenever a split leaves an odd cell, the extra cell goes
right/bottom. Always do it this way, so the composition never jitters by
a cell between neighbouring sizes.

### 1.5 Mockups

Legend: `░` liquid (glass interior / bleed background) · `█▀▄` wax (solid
style, half-blocks) · `▓` lamp metal (cap, base) · ` ` app background.
Real colours come from the palette (§5). These were drawn by a generator
script using the formulas above, so the proportions are true.

**Micro — 16×6.** Lamp only. Nothing else, ever.

```
░░░▄█▄▄░░░░░░░░░
░░░▀███▄▄██░░░░░
░░░░░██████░░░░░
░░░░▄████▀██▄░░░
░░░██░░░░░░▀░░░░
▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄
```

**Tiny — 20×8.** Full-bleed lamp plus the clock chip in the bottom-right.
No status bar, no hints.

```
░░░░▄▄▄▄░░░░░░░░░░░░
░░░░█████░▄▄▄▄░░░░░░
░░░░▀█████████░░░░░░
░░░░░░████████░░░░░░
░░░░░░█████████▄░░░░
░░░░███▀▀▀▀░▀▀█▀░░░░
░░░░▀▀░░░░░░░░░░░░░░
▄▄▄▄▄▄▄▄▄▄▄▄▄ 14:32
```

The same size with a pomodoro running. The chip switches to the
pomodoro: `▸` while running, `‖` while paused, coloured by phase (accent
for focus, `wax_hot` for breaks).

```
░░░░▄▄▄▄░░░░░░░░░░░░
░░░░█████░▄▄▄▄░░░░░░
░░░░▀█████████░░░░░░
░░░░░░████████░░░░░░
░░░░░░█████████▄░░░░
░░░░███▀▀▀▀░▀▀█▀░░░░
░░░░▀▀░░░░░░░░░░░░░░
▄▄▄▄▄▄▄▄▄▄▄ ▸ 18:24
```

**Small — 50×16.** Bleed (fewer than 20 content rows, so no glass). A
panel would leave the lamp only 56 % of the width, so the clock is a chip.
The status bar appears.

```
░░░░░░░░░░░░░▄▄▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░▄██████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░█████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░█████████░░░░░░░▄█████▄░░░░░░░░░░░░░░░░
░░░░░░░░░░░░▀████████▄▄▄▄▄▄███████░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░▀▀█████████████████░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░█████████████▀▀▀░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░████████████░░░░░░▄▄░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░███████████░░░░░█████▄░░░░░░░░░░░
░░░░░░░░░░░▄▄▄▄░░░▀██████▀▀░░░░░░██████░░░░░░░░░░░
░░░░░░░░░░█████░░░░░░░░░░░░░░░░░░░▀██▀░░░░░░░░░░░░
░░░░░░░░░░▀████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 14:32 ░
██████████████████████████████████████████████████
 ● heatmap                                 ? help
```

**Small, wide — 72×18.** The lamp keeps 69 % of the width, so a right
panel appears with the M blocks face. The panel is too narrow for the
cycle dots.

```
░░░░░░░░░░░░░░▄▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░▄██████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░██████████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░███████████░░░░░░▄▄███▄▄░░░░░░░░░░░░░░░░
░░░░░░░░░░░██████████▄▄▄▄▄█████████░░░░░░░░░░░░░░░ ▄█  █ █ ▄ ▀▀█ ▀▀█
░░░░░░░░░░░░▀▀█████████████████████░░░░░░░░░░░░░░░  █  ▀▀█ ▄ ▀▀█ █▀▀
░░░░░░░░░░░░░░░▀██████████████████▀░░░░░░░░░░░░░░░ ▀▀▀   ▀   ▀▀▀ ▀▀▀
░░░░░░░░░░░░░░░░████████████████▀░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░██████████████▀░░░▄▄▄░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░█████████████▀░░██████▄░░░░░░░░░░░ focus
░░░░░░░░░░░▄▄▄▄░▀██████████▀░░░░███████░░░░░░░░░░░ 18:24
░░░░░░░░░░██████░░░▀▀▀▀▀▀░░░░░░░▀█████▀░░░░░░░░░░░ ━━━━━━━━────────────
░░░░░░░░░▀██████░░░░░░░░░░░░░░░░░░░▀░░░░░░░░░░░░░░
░░░░░░░░░░▀▀█▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
██████████████████████████████████████████████████
 ● heatmap · lava                                       s style  ? help
```

**Medium — 80×24.** The reference size. Glass lamp, 1-row top margin.
The lamp, gutter and panel are centred as one group. Status bar on the
last row with a 2-col inset.

```

                        ▓▓▓
                       ▓▓▓▓▓
                      ▓▓▓▓▓▓▓
                      ░▄▄░░░░
                     ▄████░░░░
                     ░████░░░░
                     ░░░░░░▄▄░          ▄█  █ █ ▄ ▀▀█ ▀▀█
                    ░░░░░░████░          █  ▀▀█ ▄ ▀▀█ █▀▀
                    ░░▄▄▄▄██▀▀░         ▀▀▀   ▀   ▀▀▀ ▀▀▀
                    ░██████░░░░
                   ░░██████░░░░░
                   ░░░▀█▀▀░░▄▄▄░        focus             ●●○○
                   ░░░░░░░░░██▀░        18:24
                   ░▄██▄░░░░░░░░        ━━━━━━━━──────────────
                    ░██▀░░░░░░░
                    ███████████
                    ▓▓▓▓▓▓▓▓▓▓▓
                    ▓▓▓▓▓▓▓▓▓▓▓
                   ▓▓▓▓▓▓▓▓▓▓▓▓▓
                  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                 ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓

  ● braille · lava                 s style  c clock  p palette  ␣ pomo  ? help
```

**Large — 120×36.** L face (blocks ×2), date line, 2-row margins, every
hint.

```


                                    ▓▓▓▓▓
                                   ▓▓▓▓▓▓▓
                                   ▓▓▓▓▓▓▓
                                  ▓▓▓▓▓▓▓▓▓
                                 ▓▓▓▓▓▓▓▓▓▓▓
                                 ░░░░░░░░░░░
                                 ▄████▄░░░░░
                                 ██████▄░░░░
                                ░██████░░░░░░
                                ░░▀▀▀▀░░░░░░░                ██   ██  ██    ██████ ██████
                               ░░░░░░░░░▄▄▄▄░░             ████   ██  ██ ██     ██     ██
                               ░░░░░░░░▄█████░               ██   ██████    ██████ ██████
                               ░░░░░░░░██████░               ██       ██ ██     ██ ██
                              ░░░▄██████▀▀▀▀░░░            ██████     ██    ██████ ██████
                              ░░█████████░░░░░░
                              ░░█████████░░░░░░            thu 1 oct
                             ░░░████████▀░░░░░░░
                             ░░░░▀▀██▀▀░░░▄██▄░░
                             ░░░░░░░░░░░░░████░░           focus                         ●●○○
                             ░░░▄▄▄░░░░░░░░▀▀░░░           18:24
                              ░█████░░░░░░░░░░░            ━━━━━━━━━━━━━─────────────────────
                              ░▀███▀░░░░░░░░░░░
                               █████▄▄▄▄░░░░░░
                               ███████████████
                               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                              ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                             ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                            ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                            ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                           ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                          ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓


    ● braille · lava                        s style  c clock  p palette  f frame  l light  m minimal  ␣ pomo  ? help
```

**Wide — 160×22** (`A ≈ 3.8`). The content area is wider than 2.2:1, so
auto-frame picks **bleed**: a wide wax tank with convection cells. Glass
here would be a 17-col lamp lost in 160 cols. The right panel takes 36
cols. This terminal is short, so there's no date line, but it's wide, so
every hint shows.

```
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░███▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░█████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄████░░░░░░░░░░░░░░░░░░░░ ▄█  █ █ ▄ ▀▀█ ▀▀█
░░░░░░░░░░░░░▀▀▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░██████░░░░░░░░░░░░░░░░░░░  █  ▀▀█ ▄ ▀▀█ █▀▀
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀██▀░░░░░░░░░░░░░░░░░░░░ ▀▀▀   ▀   ▀▀▀ ▀▀▀
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄████▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░███████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄▄░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ focus                         ●●○○
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀█████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░████░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ 18:24
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀▀▀░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░ ━━━━━━━━━━━━━─────────────────────
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▄███░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░▀▀░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░
████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████████
    ● solid · ultraviolet                                                           s style  c clock  p palette  f frame  l light  m minimal  ␣ pomo  ? help
```

**Ultra-tall — 34×56** (`A ≈ 0.3`). The glass lamp is width-bound (30
cols, 2-col side clearance). A right panel won't fit, so the panel goes
*below* the lamp, as a centred block like a plinth label.

```


              ▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓
             ▓▓▓▓▓▓▓▓
            ▓▓▓▓▓▓▓▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓
           ▓▓▓▓▓▓▓▓▓▓▓▓
           ░░░░░░░░░░░░
          ░▄▄██▄▄░░░░░░░
          ████████░░░░░░
          ████████░░░░░░
         ░▀███████░░░░░░░
         ░░░▀▀▀▀▀░░░░░░░░
         ░░░░░░░░░░░░░░░░
        ░░░░░░░░░░▄█████▄░
        ░░░░░░░░░░███████░
        ░░░░░░░░░████████░
       ░░░░▄▄█████████▀▀░░░
       ░░▄██████████░░░░░░░
       ░▄███████████░░░░░░░
      ░░████████████░░░░░░░░
      ░░░███████████░░░░░░░░
      ░░░░▀███████▀░░▄▄▄▄░░░
     ░░░░░░░░░░░░░░░██████░░░
     ░░░░░░░░░░░░░░░░█████░░░
      ░░░▄▄▄▄░░░░░░░░░░░░░░░
      ░░██████░░░░░░░░░░░░░░
       ░██████░░░░░░░░░░░░░
       ░▄████▄░░░░░░░░░░░░░
        ████████▄▄▄▄▄▄▄▄▄▄
        ██████████████████
        ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
       ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
      ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
      ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
     ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
    ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
   ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓




       ▄█  █ █ ▄ ▀▀█ ▀▀█
        █  ▀▀█ ▄ ▀▀█ █▀▀
       ▀▀▀   ▀   ▀▀▀ ▀▀▀


       focus           ●●○○
       18:24
       ━━━━━━━━────────────



 ● solid                   ? help
```

---

## 2. Lamp viewport

### 2.1 Frame: glass vs bleed

`frame = auto | glass | bleed` (key `f` cycles). **auto** resolves to:

* **glass** if `content_rows ≥ 20` **and** content-area visual aspect `A ≤ 2.2`
* **bleed** otherwise (tiny/small windows and very wide strips)

Minimal mode uses the same rule.

**Glass silhouette.** Stylised, chunkier than a real lamp (a real one is
about 3.2:1; ours is **2.5:1** total height : max width), so it still
reads as a lamp at 20 rows.

| Part | Share of lamp height `Ht` | Width (fraction of `W = 0.8 × Ht` cols) |
|---|---|---|
| Cap | 15 % (≥ 2 rows) | 0.18 at top → 0.40 at bottom (truncated cone) |
| Bottle | 63 % | 0.40 at top → **0.78 bulge at 72 % down** → 0.56 at bottom |
| Base | 22 % (≥ 2 rows) | 0.56 at top → 1.00 at bottom (flared cone) |

Rendering rules for the silhouette:

* **The glass has no outline.** It's the `liquid` colour against `bg`.
  Edges use half-column precision (`▐` `▌` in the liquid colour), so the
  taper is smooth at any width and odd/even widths both centre exactly.
* Cap and base are drawn in `metal`, shaded top-light → bottom-dark (two
  tones lerped from `metal`). Slopes use `◢◣` / half-blocks, never `/\`.
* A one-cell **highlight streak** runs down the bottle's left side at
  ~20 % toward `text`. It's shown only when lighting is on.
* With lighting on, the base casts a warm falloff into the bottom third
  of the liquid. The heat source is visibly the base.
* In 16-colour / NO_COLOR mode there's no liquid tint, so the bottle gets
  a thin `▕` … `▏` edge in `metal` instead (§5.3).

**Bleed.** The lamp region *is* the tank: heat source along the bottom
row, cooling at the top, no metal. Bleed ignores outer margins. Edge to
edge is the point.

### 2.2 Proportions: square sim pixels

Terminal cells are about 1:2 (w:h). The renderer never samples one value
per cell and stretches it. Every style samples on a grid of **square
pixels**:

| Style family | Sub-samples per cell | Pixel grid for a cols×rows region |
|---|---|---|
| half-block (solid, heatmap, dither, …) | 1 × 2 | cols × 2·rows |
| braille | 2 × 4 | 2·cols × 4·rows |
| ASCII / glyph (ascii, matrix, topo, …) | 1 × 1, sample at the cell centre, aspect-corrected | cols × rows, `y` scaled by `cell_aspect` |

The sim lives in **world units**, independent of the terminal:

* World height is always `1.0`. World width is `A_region` (the visual
  aspect of the lamp region; for glass, the bottle's bounding box).
* Blob radii, velocities and the heat field are in world units. A
  terminal resize changes **sampling density only**, never the physics.
  A blob that's 10 % of lamp height stays 10 % at 30 rows or 120.
* Glass: world shape = the bottle profile above (the walls are the
  bottle's curved sides), with a fixed aspect. The sim never resizes in
  glass mode, only the sampling does.
* Bleed: world width follows the region. On resize the walls **ease** to
  the new width over 250 ms (the sim pushes blobs, so nothing teleports),
  and total wax volume is kept at a constant **≈ 22 % of world area** by
  slowly growing/shrinking the bottom pool (no blobs pop in or out).

### 2.3 Cell aspect

`cell_aspect` = from `crossterm::terminal::window_size()` pixel fields
when they're non-zero (`(px_h/rows) / (px_w/cols)`, clamped 1.6–2.6),
otherwise `display.cell_aspect` from config, otherwise **2.0**. Recompute
on every resize.

### 2.4 Resolution scaling & budget

* The field is sampled at the style's native pixel grid (table above), up
  to a **budget of 400 k samples/frame**. Above that the renderer samples
  at a reduced grid and bilinearly upsamples the field (not the glyphs).
  That only happens at braille + huge sizes.
* Sampling culls per blob: each blob only touches pixels inside its
  influence box. Cost scales with *blob area*, not blobs × pixels.
* Blob count is set by the world, not the window: glass has **6–10**
  blobs. Bleed has `≈ 5 × A_region` blobs, clamped to 4–28.

---

## 3. Minimal mode

`m` toggles it; `--minimal` (short `-m` to be added) starts in it; it's persisted in config.
The switch is instant: the next frame shows the new layout, and the sim is
untouched (same blobs, same phase).

* **Just the lamp.** No status bar, no panel, no hints, no borders. Glass
  is centred (bleed fills the screen), with the same frame auto-rule.
* **Tiny optional clock** (`minimal.clock = "under" | "corner" | "off"`,
  default `under`): `14:32` in `dim`, centred on the last row under the
  lamp base. In bleed, or when there's no spare row, it becomes the
  corner chip. A running pomodoro replaces it with `▸ 18:24` in the phase
  colour.
* Every key still works. Toasts still appear (that's the only feedback
  minimal mode gives). `?` still opens help.
* Pomodoro phase changes still flash (§4.4).

**Minimal — 80×24:**

```

                                      ▓▓▓
                                     ▓▓▓▓▓
                                    ▓▓▓▓▓▓▓
                                    ░▄▄░░░░
                                   ▄████░░░░
                                   ░████░░░░
                                   ░░░░░░▄▄░
                                  ░░░░░░████░
                                  ░░▄▄▄▄██▀▀░
                                  ░██████░░░░
                                 ░░██████░░░░░
                                 ░░░▀█▀▀░░▄▄▄░
                                 ░░░░░░░░░██▀░
                                 ░▄██▄░░░░░░░░
                                  ░██▀░░░░░░░
                                  ███████████
                                  ▓▓▓▓▓▓▓▓▓▓▓
                                  ▓▓▓▓▓▓▓▓▓▓▓
                                 ▓▓▓▓▓▓▓▓▓▓▓▓▓
                                ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
                               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓

                                     14:32
```

---

## 4. Chrome: status bar, toasts, help, pickers

### 4.1 Status bar

One row, the last row of the terminal. **It's a whisper, not a bar:** no
background fill, no reverse video, no separators other than spacing.
Inset `max(1, horizontal margin)` cols on each side.

```
  ● braille · lava                 s style  c clock  p palette  ␣ pomo  ? help
  └─ left ────────┘                └─ right: hints ─────────────────────────┘
```

* **Left:** `●` in `accent`, then the style name in `text`, then
  `· palette` in `dim` (only if `cols ≥ 60`). When paused (`z`), `●`
  becomes `‖` and the text says `frozen`.
* **Centre:** empty, unless debug HUD (`d`) is on: `60 fps · 2.1 ms · 412k
  px` in `dim`.
* **Right:** hints in `dim`, each formatted `key label` with the key in
  `text`. The full list in display order is `s style  c clock  p palette
  f frame  l light  m minimal  ␣ pomo  ? help`. The bar fits as many as
  possible while keeping a gap of at least 4 cols to the left segment.
  Hints drop in this order: `m`, `l`, `f`, `␣`, `p`, `c`, `s`. `? help`
  always goes last.
* The pomodoro is **not** repeated in the status bar. It lives in the
  panel or chip.

### 4.2 Toasts

* Appear centred in the **top row of the lamp region** (glass: just above
  the cap; bleed: row 0), with a 1-cell `bg` pad on each side.
* Format: `‹name›  ‹i›/‹n›` for cycling (`braille  6/12`), or a short
  lowercase sentence (`press r again to reset`).
* Last 1.4 s. The final 400 ms fade `text`→`bg` in truecolor. In 256 and
  16 colour they just vanish. A new toast replaces the old one
  immediately; toasts never stack.

### 4.3 Help overlay (`?`)

* **Medium and up:** a centred sheet, `min(64, cols−4)` × `min(18,
  rows−2)`, **rounded border in `metal`**. Overlays are the only place
  borders appear. The title `keys` sits in the top border in `accent`,
  and `esc close` in the bottom-right border in `dim`. Two columns: *lamp*
  | *clock & pomodoro* + *app*, with section headers in `dim`, keys in
  `accent`, labels in `text`.
* The lamp keeps animating behind it, dimmed to 35 % (truecolor: lerp
  toward `bg`; 256/16: the sheet's rect is cleared to `bg`, the rest
  isn't dimmed).
* **Below Medium:** a full-screen sheet, one column, scrollable with
  `j/k/↑/↓`, no border. In Micro it shows the single line `? help · q
  quit · m mode`, clipped by dropping items.
* `?`, `esc` or `q` closes it. While help is open, `q` closes help and
  does *not* quit.

```

                      ···
        ╭─ keys ───────────────────────────────────────────────────────╮
        │                                                              │
        │  lamp                          clock & pomodoro              │
        │  s  next style                 c  next face                  │
        │  S  style picker               C  face picker                │
        │  p  next palette               t  show/hide clock            │
        │  P  palette picker             T  12h / 24h                  │
        │  f  frame: auto/glass/bleed    ␣  start / pause              │
        │  l  lighting                   n  skip phase                 │
        │  [ ]  heat − +                 r r  reset pomodoro           │
        │  - +  speed                                                  │
        │  z  freeze                     app                           │
        │  0  reset heat & speed         m  minimal   b  status bar    │
        │  R  reseed wax                                               │
        │                                                              │
        │                                                              │
        │  ?  this help   q  quit   d  debug hud   ctrl-l  redraw      │
        ╰─────────────────────────────────────────────── esc close ────╯
                ···············
               ·················


```

### 4.4 Pickers (`S` style, `C` face, `P` palette)

* **Live preview:** moving the cursor applies the item to the live lamp
  or clock right away. `⏎` keeps it, `esc` reverts to what was active
  when the picker opened.
* **Medium and up:** a right-anchored sheet (it covers the panel; the
  lamp stays visible and *un*-dimmed, because the point is to watch it
  change). Size: width 26, height `items + 6` (capped, scrolls). Rounded
  `metal` border, title in `accent`, cursor `▸` + name in `accent`, the
  active item marked with `·` after its name.
* **Small:** a bottom sheet across the full width, taking up to half the
  height.
* **Tiny / Micro:** an inline selector in the top row, `‹ braille ›`. Use
  `←/→` or `h/l` (also `j/k`).
* Keys inside a picker: `↑↓`/`j k` move, `1`–`9` jump, `⏎`/`space` keep,
  `esc` revert. Pressing the opening key again keeps and closes.
* The status bar's right side switches to picker hints: `↑↓ preview  ⏎
  keep  esc revert`.

```

                      ▓▓▓
                     ▓▓▓▓▓                          ╭─ style ────────────────╮
                    ▓▓▓▓▓▓▓                         │                        │
                    ░▄▄░░░░                         │   solid                │
                   ▄████░░░░                        │   outline              │
                   ░████░░░░                        │   heatmap              │
                   ░░░░░░▄▄░                        │   ascii                │
                  ░░░░░░████░                       │   dither               │
                  ░░▄▄▄▄██▀▀░                       │ ▸ braille              │
                  ░██████░░░░                       │   halftone             │
                 ░░██████░░░░░                      │   crt                  │
                 ░░░▀█▀▀░░▄▄▄░                      │   synthwave            │
                 ░░░░░░░░░██▀░                      │   matrix               │
                 ░▄██▄░░░░░░░░                      │   topo                 │
                  ░██▀░░░░░░░                       │   chrome               │
                  ███████████                       │                        │
                  ▓▓▓▓▓▓▓▓▓▓▓                       │ ⏎ keep   esc revert    │
                  ▓▓▓▓▓▓▓▓▓▓▓                       │                        │
                 ▓▓▓▓▓▓▓▓▓▓▓▓▓                      ╰────────────────────────╯
                ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓
               ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓

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
 18:24                    ← remaining (text; accent while running)
 ━━━━━━━━──────────────   ← progress: ━ accent, ─ dim
```

When the pomodoro is idle, the panel shows `focus  25:00` dim, with an
empty bar. The pomodoro is always present in the panel, so it's always
discoverable.

**Face variants.** Every face declares variants with fixed sizes, and the
panel uses the largest one that fits its inner rect. Anything that can't
fit falls back to `text` (`14:32`, 5×1), which always fits.

| Face | S | M | L | XL |
|---|---|---|---|---|
| `blocks` (default) | — | 3×5 font, half-blocks: 19×3 | ×2: 37×5 | ×3 (Huge only): 55×8 |
| `segment` | 13×1 (`▖▗` mini) | 19×3 | 31×5 | 45×7 |
| `analog` | — | 15×8 | 23×12 | 31×16 (circle aspect-corrected) |
| `binary` | 11×4 | 17×6 | — | — |
| `words` | 18×3 | 24×4 | 32×4 | — |
| `text` | 5×1 | — | — | — |

The colon never blinks (motion belongs to the lamp). Seconds appear only
in L/XL variants and the `text` face's 12h/24h follows `T`.

**Phase-change flash.** When a pomodoro phase ends: the lamp's `metal`
parts (or, in bleed, the liquid tint) pulse toward `accent` once over
600 ms, a toast says `break · 5:00`, and the terminal bell sounds if
`pomodoro.bell = true` (default true).

---

## 5. Palettes / lamp themes

### 5.1 Roles

Every palette defines exactly these nine roles. Nothing outside `ui/`
and `render/` hard-codes a colour.

| Role | Used for |
|---|---|
| `bg` | app background outside the glass |
| `liquid` | glass interior / bleed background |
| `wax_cool` `wax_mid` `wax_hot` | 3-stop temperature gradient (cool → hot). Single-colour styles use `wax_mid`, or lerp by temperature |
| `metal` | cap, base, overlay borders |
| `text` | primary text |
| `dim` | secondary text, hints, idle states |
| `accent` | **the one accent**: selection cursor, running pomodoro, `●`, toast keys |

### 5.2 The palettes

Default: **lava**. Names are lowercase in the UI. 256 = xterm index
(nearest by weighted RGB, hand-fixed where two roles collided). 16 =
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
| dim | `#776E93` | 96 | DarkGray |
| accent | `#A78BFA` | 141 | LightBlue |

**abyss**: deep sea. Teal wax glowing to seafoam in navy water.

| role | hex | 256 | 16 |
|---|---|---|---|
| bg | `#060B10` | 232 | default |
| liquid | `#0B1A24` | 234 | default |
| wax_cool | `#0B4F6C` | 23 | Blue |
| wax_mid | `#1A9BA8` | 31 | Cyan |
| wax_hot | `#A8F5E4` | 158 | LightCyan |
| metal | `#34495A` | 238 | DarkGray |
| text | `#D6E7EE` | 254 | default |
| dim | `#5F7785` | 66 | DarkGray |
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

**mono**: graphite. Grayscale only. It suits dither/halftone/braille and
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
| metal | `#A8997E` | 138 | Gray |
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
| truecolor | continuous lerp across the 3 wax stops | `bg` painted (unless `theme.transparent = true`) | fades, dimming, glow all on |
| 256 | lerp in RGB, then quantise each cell to the nearest xterm index (cached LUT, 64 steps) | `bg` painted (index above) | toast fade → instant; help dim → cleared rect |
| 16 | 3 discrete steps; styles add glyph density (`░▒▓█`) to show temperature | always `default` | glass gets a thin `▕ ▏` edge in `metal`; lighting adds density, not colour |
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
| `esc` | close overlay / cancel picker | no-op otherwise: **esc never quits** (esc is muscle memory for "close this"; an accidental quit loses pomodoro state). The scaffold currently quits on esc, and `lava-xxx` must change that |
| `m` | minimal mode on/off | also `--minimal` (add short `-m`) |
| `b` | status bar on/off | full mode only |
| `s` / `S` | next style / style picker | toast shows `name  i/n` |
| `c` / `C` | next clock face / face picker | |
| `p` / `P` | next palette / palette picker | |
| `f` | frame: auto → glass → bleed | toast shows the *resolved* frame (`auto · glass`) |
| `l` | lighting on/off | |
| `t` | clock shown/hidden | hides the face in the panel/chip; the pomodoro stays |
| `T` | 12h / 24h | |
| `space` | pomodoro start / pause / resume | starts a focus phase if idle |
| `n` | pomodoro: skip to next phase | |
| `r` | pomodoro reset (press **twice** within 2 s) | first press toasts `press r again to reset` |
| `[` / `]` | heat − / + (5 steps, default middle) | more heat = more, faster blobs; toast shows `heat ▮▮▮▯▯` |
| `-` / `+` (`=`) | sim speed ×0.25 · ×0.5 · ×1 · ×2 · ×4 | toast `speed ×2` |
| `0` | reset heat and speed | |
| `z` | freeze / unfreeze the lamp | frozen = zero sim cost; the clock keeps ticking |
| `R` | reseed the wax (new random seed) | blobs dissolve into the pool, then rise anew over ~2 s, never a hard cut |
| `d` | debug HUD (fps, frame ms, samples) | |
| `ctrl-l` | force full redraw | |

### 6.2 In overlays

| Context | Keys |
|---|---|
| help | `j k ↑ ↓` scroll · `?` `esc` `q` close |
| picker | `j k ↑ ↓` move (live preview) · `1`–`9` jump · `⏎` `space` keep · `esc` revert · opening key = keep + close |
| tiny inline picker | `h l ← →` (also `j k`) move · `⏎` keep · `esc` revert |

Every key not listed is ignored (no beep, no toast). Overlay keys take
precedence over global keys; global keys other than `ctrl-c` don't fire
while an overlay is open.

### 6.3 Mouse

`input.mouse = false` by default, because mouse capture breaks the
terminal's native text selection. When it's on: click/drag on the lamp
= a local heat pulse (the wax there warms and rises), scroll in pickers
and help, click a picker item to preview, double-click to keep.

---

## 7. Motion & performance

| Target | Value |
|---|---|
| Sim timestep | fixed `SIM_HZ` (**120 Hz** in the scaffold, `timing::FixedStep`, max 8 steps/frame), decoupled from render; render interpolates with `alpha()` |
| Render rate | default **60 fps** (`--fps 1..=240`, `display.fps`). Lava is slow, but 60 fps keeps input feeling instant and makes the slow motion buttery |
| Wax tempo (×1, heat 3) | a blob takes **~20–40 s** to cross the lamp: slow, hypnotic, never jittery |
| Startup → first frame | **< 100 ms**. The sim starts *pre-warmed*: ~600 headless steps at launch, so frame 1 already looks alive (no 2-hour warm-up) |
| Input latency | key → visible change **≤ 1 frame** (≤ 17 ms at 60 fps). The loop blocks on `event::poll(time_to_next_frame)`; any input that changes UI state triggers an immediate redraw, without waiting for the tick |
| Frame CPU (release, 2020-era laptop) | ≤ **2 ms** at 80×24; ≤ **8 ms** at 200×60 with braille (≤ 50 % of a 60 fps budget) |
| CPU usage | ≤ **5 %** of a core at 80×24, ≤ **15 %** at 200×60 @ 60 fps |
| Output bandwidth | rely on ratatui's cell diff; ≤ ~200 KB/s at 80×24 (SSH-friendly) |
| Unfocused | on `FocusLost` (if the terminal reports it), drop to **10 fps**; back to normal on `FocusGained` |
| Frozen (`z`) | no sim steps, redraw only on clock-minute change / input |

**Resize behaviour.**

* Handle `Event::Resize` at once: recompute `layout()`, re-derive
  `cell_aspect`, and draw the new geometry on the **next frame**. Never
  draw a frame with stale geometry, and do one full clear + repaint.
* Glass: the sim is untouched (only sampling changes), so the lamp just
  gets crisper or coarser.
* Bleed: walls ease to the new width over 250 ms (§2.2).
* Coalesce resize storms: when multiple resize events arrive in one poll
  batch, only the last one counts.

**Adaptive quality** (silent; it never changes the user's choices):

1. If the moving-average frame time is > 80 % of budget for 2 s, the
   sampling grid drops one step (e.g. braille samples at half resolution
   and upsamples).
2. Still over budget: fps 60 → 30 (never below 30 from adaptation alone).
3. Recovers in reverse once it's < 40 % of budget for 5 s.

The debug HUD shows when this is active (`fps` turns `wax_hot`).

---

## 8. Visual design principles

1. **The lamp is the hero; everything else whispers.** Chrome uses `dim`
   and `text` only, with no fills, bars or boxes in the resting state.
2. **Hide before you cram.** Elements drop out whole, in the fixed
   priority order (§1.3). Nothing truncates mid-word, wraps or overlaps.
3. **One accent colour**, used sparingly: cursor, running pomodoro,
   status `●`, help keys. If two things are accented, one of them
   shouldn't be.
4. **No borders at rest.** Separate things with whitespace. Rounded
   borders appear only on transient overlays (help, pickers).
5. **Only the lamp moves.** No spinners, no blinking colon, no animated
   chrome. The exceptions are the pomodoro bar's progress, toast fades and
   the phase-change flash, all of which carry information.
6. **Centred, proportional compositions.** Margins and gutters scale with
   the window. Odd leftover cells always go right/bottom, so nothing
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
frame = "auto"           # auto | glass | bleed
lighting = false
heat = 3                 # 1..5
speed = 1.0              # 0.25 | 0.5 | 1 | 2 | 4

[theme]
palette = "lava"
transparent = false      # true = never paint bg outside the glass

[clock]
face = "blocks"
show = true
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
clock = "under"          # under | corner | off

[input]
mouse = false
```

CLI: `--minimal`, `--fps <n>` (exist today; add short `-m`, plus hidden `--frames`),
and planned `--style <name>`, `--palette <name>`, `--color <depth>`,
`--seed <u64>`. Flags override config for the
session only. They're never written back.

---

## 10. Out of scope for v1 (ideas, not commitments)

* **Lamp shelf:** on ultra-wide screens in glass mode, 2–3 lamps side by
  side, each with its own seed and palette.
* Kitty/sixel graphics backend for true-pixel wax.
* Ambient mode: auto-cycle styles/palettes every N minutes.
