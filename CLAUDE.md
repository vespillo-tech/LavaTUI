# Project Instructions for AI Agents

This file provides instructions and context for AI coding agents working on this project.

<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:1105d646 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/core-concepts/sync-concepts.md for details and anti-patterns.

## Agent Context Profiles

The managed Beads block is task-tracking guidance, not permission to override repository, user, or orchestrator instructions.

- **Conservative (default)**: Use `bd` for task tracking. Do not run git commits, git pushes, or Dolt remote sync unless explicitly asked. At handoff, report changed files, validation, and suggested next commands.
- **Minimal**: Keep tool instruction files as pointers to `bd prime`; use the same conservative git policy unless active instructions say otherwise.
- **Team-maintainer**: Only when the repository explicitly opts in, agents may close beads, run quality gates, commit, and push as part of session close. A current "do not commit" or "do not push" instruction still wins.

## Session Completion

This protocol applies when ending a Beads implementation workflow. It is subordinate to explicit user, repository, and orchestrator instructions.

1. **File issues for remaining work** - Create beads for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **Handle git/sync by active profile**:
   ```bash
   # Conservative/minimal/default: report status and proposed commands; wait for approval.
   git status

   # Team-maintainer opt-in only, unless current instructions forbid it:
   git pull --rebase
   git push
   git status
   ```
5. **Hand off** - Summarize changes, validation, issue status, and any blocked sync/commit/push step

**Critical rules:**
- Explicit user or orchestrator instructions override this Beads block.
- Do not commit or push without clear authority from the active profile or the current user request.
- If a required sync or push is blocked, stop and report the exact command and error.
<!-- END BEADS INTEGRATION -->


## Project: LavaTUI

A terminal lava lamp. Fluid, physically-plausible wax blobs (heat rises,
cools, sinks, merges, splits) drawn in many swappable render styles
(solid, outline, ASCII, braille, halftone, synthwave, …), filling the
window edge to edge (no lamp silhouette, no lighting: both were dropped in
v1.1, along with the heatmap, dither and crt styles). Ships with a selectable-style clock and a
pomodoro timer, a full TUI (panels, status bar, help, keybinds) and a
minimalist "just the lamp" mode. It's a "vibe app": looks and design matter
as much as the code.

### Stack

- **Rust** (stable, edition 2024) — single binary crate `lavatui`
- **ratatui** + **crossterm** for terminal UI / input
- **serde** + **toml** for config (XDG config dir via `directories`)
- **jiff** for local wall-clock time (clock faces, date line)
- Tests: built-in `cargo test`; pure logic (sim, clock, pomodoro) kept
  terminal-free so it is unit-testable

## Build & Test

```bash
cargo build                          # debug build
cargo run --release                  # run the lamp (release: the sim wants the speed)
cargo run --release -- -m            # just the lamp, no chrome (--minimal)
cargo run --release -- --fps 30      # target render fps (1..=240, default 60)
cargo run --release -- --config /tmp/x.toml   # use a scratch config file
cargo run --release -- --frames 300  # hidden: exit after N frames (smoke test / timing)
cargo test                           # unit tests (sim, render, layout sweep, keymap, model, config)
cargo fmt --check                    # formatting gate
cargo clippy --all-targets -- -D warnings   # lint gate
cargo test --release -- --ignored --nocapture bench_fill   # field sampler + step timing
cargo test --release -- --ignored --nocapture bench_lamp   # per-style frame time + bytes/frame
UPDATE_SNAPSHOTS=1 cargo test        # rewrite render + layout snapshots (review the diff!)
```

The binary needs a real TTY (it errors out cleanly without one). To smoke-test
headlessly, run it under a pty with a window size set (e.g. Python `pty.fork`
+ `TIOCSWINSZ`) and `--frames N`; `script` alone gives a 0x0 pty. Keep
draining the pty until the child exits, or it blocks writing and never
reads your quit key; on macOS a read on the master after exit is EOF/EIO.
`tools/trace_frames.py --output /tmp/lavatui-traces` runs five-minute
sized pty sessions and summarizes frame intervals/spike locations; see
`docs/perf/frame-trace.md` for the trace columns and measurement limits.
`docs/screenshots/capture.py` does exactly this (pyte + Pillow) and
regenerates the README screenshots; rerun it after visible changes.
`README.md` is the user-facing overview (features, keys, config, perf
numbers); `docs/design.md` is the layout/visual contract.

## Architecture Overview

- `main.rs`   — parse CLI, `ratatui::try_init` (raw mode, alt screen, panic
                hook that restores the terminal), run app, `ratatui::restore`.
- `cli.rs`    — clap derive flags (`-m/--minimal`, `--fps`, `--style`,
                `--palette`, `--color`, `--seed`, `--config`, hidden
                `--frames`, `--panic-after`) → `config::Session` (session-only overrides).
- `config/`   — `Settings`: the persisted TOML surface of design §9 (serde,
                every field defaulted, `sanitized()` clamps). `Session` layers
                CLI flags on top; `to_persist` puts the file's values back for
                fields a flag still holds, so flags are never written back.
                `store.rs`: XDG path (`$XDG_CONFIG_HOME/lavatui/config.toml` or
                the `directories` config dir), load (missing → defaults; a bad
                value is ignored with a toast and the rest kept; a syntax
                error → defaults + toast), save (anything a save would drop
                is first copied to `config.toml.bak`; saves keep comments,
                follow symlinks and are atomic). Settings older versions had
                (`RETIRED_KEYS`: `lamp.frame`, `lamp.lighting`, `clock.show`)
                load without a toast and a save takes them out
                (`clock.show = false` carries over as `dock.clock = "off"`;
                `Parsed::baseline` makes the next save write it); a removed
                style name (`RETIRED_STYLES`) quietly becomes `solid`.
                `[dock]` is `dock::DockSettings`: `anchor` plus one
                `<widget name> = side|overlay|off` per registered widget
                (a flattened map, so a new widget needs no config code).
- `app/`      — `mod.rs` is the loop only: poll input until the frame
                deadline → `Model::update(action)` (any input draws at once;
                queued events are drained first, as one burst that
                `replies.rs` strips of terminal replies (DCS/OSC/APC, DA2
                tails) crossterm reads as keys; the wait is recomputed from
                the deadline each burst; `Events` carries the clock, so
                tests run on a fake one) → inside `terminal.draw`:
                `Model::tick(now, frame.area(), local_time)` → `ui::draw`
                (tick and draw always share the drawn size; `ui::draw` also
                relayouts if they ever differ). It owns the terminal, reads
                local time (jiff, + `SystemTime` for sleep detection) and
                cell aspect (`window_size` pixels), rings the bell, enables
                focus reports (+ mouse capture if `input.mouse`) behind a
                Drop guard; `main`'s chained panic hook turns them off too.
                ctrl-l repaints via `Terminal::resize`, never
                `Terminal::clear` (that blocks on a cursor-position query).
                `model/`: all state (settings, world, style/theme/face,
                pomodoro, overlay, toast, layout) and all behaviour.
                `mod.rs`: the state, queries and the per-frame `tick`;
                `actions.rs`: `update` applies one `Action` (overlay keys
                first; under an overlay only quit/resize/focus get through),
                `global_action` is one exhaustive match, one arm per action,
                onto small helpers (`toggle` for any bool setting);
                `pickers.rs`: `PickerKind`/`Picker`, picker keys and clicks,
                live preview / keep / revert. `tick` advances
                pomodoro/toasts/flash/eased speed/sim steps,
                recomputes the layout, feeds the lamp's aspect to the sim,
                and queues the debounced (1 s) save. `model/saving.rs` owns
                the config Store on a worker: frames only try-send/try-receive;
                quit flushes the last change and joins after drawing stops.
                `output.rs` batches each frame (including resize clears) in
                one reusable byte buffer, wrapped in DEC 2026 synchronized
                updates (passthrough on legacy non-ANSI Windows consoles);
                panic/error cleanup also ends synchronization and shows the cursor.
                `--trace <path>` / `LAVATUI_TRACE` records frame CSV in memory
                and writes it on normal/error exit (not panic). Fps: 10 unfocused; frozen
                frames sleep until the clock / pomodoro readout changes
                (`idle_until`); `frame_drawn` feeds adaptive quality.
- `timing.rs` — pure loop timing: `FixedStep` (accumulator, no per-frame
                cap: sim time tracks real time × speed at any fps; only a
                > 1.5 s `STALL` is cut short; `alpha()` for interpolation),
                `FramePacer` (fixed-grid frame deadlines, skips expired slots
                when late; immediate input redraws keep the grid),
                `FpsMeter` (EMA), `Quality` (§7 adaptive quality: reduced
                sample grid, then half fps; recovers with hysteresis and
                backoff so it never flaps).
- `sim/`      — wax simulation (pure, seeded, deterministic). `World::new(seed,
                aspect)` + `step(dt)` at the fixed `dt` (`SIM_HZ = 120`).
                World units: height 1, width = visual aspect, x centred on 0;
                a straight-walled tank whose walls ease to a new width.
                Pool on the heater buds blobs; heat/buoyancy/drag/cohesion,
                merge + split, melt back into the pool; wax area conserved.
                `field.rs`: `Field::prepare(&world, alpha)` once per frame,
                then `fill(&mut [Sample], cols, rows)` / `sample(u, v)`
                (v down; density `>= SURFACE` is wax). Kernels are built per
                fill for its pixel size: lobes fade out on blobs only a few
                pixels across and the pool is drawn >= `MIN_POOL_PIXELS`
                deep. Randomness: `rng.rs` (`Rng`, stateless `hash`).
                `controls.rs`: heat, reseed, heat pulse, `SimSpeed`. Model
                notes and all tuning constants are at the top of `sim/mod.rs`
                (incl. `WAX_TEMP`, the span renderers map onto wax colours).
                Accessors only tests read are `#[cfg(test)]`.
- `theme/`    — palettes + colour depth: the only place colours are decided.
                `Palette` (9 `Role`s × 8 palettes from design §5.2, hex/256/16),
                `ColorDepth::detect()` (NO_COLOR → COLORTERM → TERM, §5.3),
                `Theme::new(palette, depth)`; `theme.with_role(role, paint)`
                repaints one role (ramps follow), e.g. the phase-change flash
                (liquid toward accent). Styles ask for `Ink::Role(r)` or
                `Ink::Wax(t)` (cool→mid→hot) via
                `theme.color(ink)` / `theme.paint(ink).mix(..).scale(..).color()`;
                16/none never blend (dominant side wins), 256 snaps to xterm.
                `theme.background(transparent)` is the app background (`bg`,
                or `TERMINAL_DEFAULT` when `theme.transparent`): every bit of
                chrome uses it, so transparent paints no `bg` anywhere. `fade_to_bg` dims what
                a sheet covers. No `Color::` outside `theme/` except
                `render/cell.rs` and tests. The paint path's small helpers
                are `#[inline]` (they sit in every style's pixel loop);
                `fallback` deliberately isn't (see its doc).
- `render/`   — render pipeline. `LampView { field, style, theme, time,
                options }` is a `StatefulWidget` (`LampOptions`: `reduced`
                grid for adaptive quality; state
                `LampState` = reused scratch buffers); it samples the field
                at the style's `Grid` (half-block 1×2, braille 2×4, …; >400k
                samples → coarse fill + bilinear upsample), then calls the
                style's `draw(&Canvas, buf)`.
                In 256-colour mode a final pass (`render/dither256.rs`) turns
                blended RGB into xterm indices, Bayer-dithering dark tints the
                cube lacks (`Theme::dithering`/`Theme::dither`).
                A style is a unit struct implementing `LampStyle` (`NAME`,
                `GRID` consts + `draw`), one per file in `render/styles/`,
                listed in `styles::ALL` as `StyleEntry::of::<S>()` (cycle
                order; `StyleId` looks up by name; `styles::ALIASES` maps
                old names, e.g. `glass` → `chrome`). `canvas.rs`: `Canvas`
                (samples, theme, time, its `area`) and the
                shared cell loops: `for_each_cell(buf, |at, cell|)` (an
                `At` carries the cell, its top-left pixel and the liquid's
                colour, `base`; `LIQUID` is the ink behind the wax), `draw_half_blocks(buf, |x, y| Option<Color>)`,
                `cell_at` / `cell_mut` for styles that walk their own order
                (matrix, column by column). `cell.rs`: `half_block`,
                `braille_dots(cx, cy, |x, y| bool)`, `blank` / `glyph` /
                `mark`. Level helpers in `mod.rs`: `coverage` (quantised AA
                edge), `soft_edge`, `wax_heat`, `bayer`,
                `smoothstep`; in `styles/mod.rs`: `is_edge`, `quantise`,
                `stepped_heat` (16 wax steps), `hash`. Snapshots:
                `render/snapshots/` (`UPDATE_SNAPSHOTS=1 cargo test` to rewrite, then review).
- `clock/`    — clock faces (`Face` trait + `FACES` registry: blocks, segment,
                analog, binary, words, text; each lists fixed-size `Form`s and
                `fit()` picks the largest that fits) and the pomodoro state
                machine (`Pomodoro`, pure, `Instant` passed in) +
                `PomodoroWidget`. Faces leave spaces transparent.
- `dock/`     — the widget dock (design §4.6): `DockWidget` trait (`name`,
                `default_place`, `forms(model, place)` → fixed-size
                `WidgetForm`s most preferred first, with `Needs` (huge /
                tall terminal) and `fill`; `draw(model, form, rect, Look)`;
                `chip(model)` → one-line fallback with a rank) + `WIDGETS`
                registry (`clock.rs`, `pomodoro.rs`). `Place` (side /
                overlay / off), `Anchor`. Widgets are stateless views of the
                `Model`. Seconds never on the lava; date forms need tall.
- `ui/`       — the only terminal-facing code. `layout.rs`: the pure
                `layout(area, &LayoutInput) -> Layout` of design §1 (the lamp
                rect, right/bottom panel = `Stack` of side widgets, `on_lava`
                = `Stack` of overlay widgets at the anchor (≤ 60 % × 50 % of
                the lamp, backing ≤ 35 % of its area, lamp ≥ 28×10, clear of
                toast/chip rows), chip for the top-ranked widget with no
                room, status row, toast row). `LayoutInput.dock` is one
                `DockItem` (place, forms, chip width/rank) per widget;
                `first_fit` tries form combinations in hide order (last
                widget shrinks first; date → face size → stack → chip). `keymap.rs`: the single `KEYMAP` table that drives
                both dispatch (`action_for(event, InputMode)`) and the help
                overlay. `mod.rs` draws back to front; `dock.rs` (panel, widgets on the
                lava over their soft backing: veiled 82 % to liquid, fading
                out over 1½ rows, truecolor only — plain liquid below it —
                and the chip), `chrome.rs` (status
                bar + hint fitting, HUD, toasts), `help/` (`sheet.rs`: the
                pure geometry the model also reads — form per size, lines,
                body rect, `footprint`, `max_scroll`; `mod.rs` draws),
                `picker.rs` (`placement`/`hit`: geometry shared by draw and
                mouse).
                Chrome never shares cells: `ui::draw` leaves out whole any
                panel/lava stack/chip/toast/HUD an overlay (or a toast)
                would touch.
                `render_tests.rs`: whole frames via `TestBackend` at the
                mockup sizes (help, pickers, toasts, HUD, minimal), lamp
                cells printed `~`; snapshots `ui/snapshots/render_*.txt`.
                `tests.rs`: size sweep 1×1..300×100 × 8 setting variants
                plus 12×5..300×90 × 35 lava/side/anchor mixes (no
                overlap/overflow, lamp always there, lava limits, right chip)
                + mockup-size checks + layout snapshots in `ui/snapshots/`.

crossterm is used via ratatui's re-export (`ratatui::crossterm`) so the two
never drift apart; there is no direct crossterm dependency.

## Adding things

- **A render style**: `render/styles/<name>.rs` with `pub struct X;` and
  `impl LampStyle for X { const NAME; const GRID; fn draw(c, buf) }`;
  draw with `c.draw_half_blocks` (half-block pixels) or `c.for_each_cell`
  (+ `cell::braille_dots` / `mark`), colours only via `c.theme`. Then
  `mod <name>;` and one `StyleEntry::of::<<name>::X>()` line in
  `styles::ALL`. The style tests (every style at every depth, snapshots,
  stays-in-area, time-purity) pick it up; run
  `UPDATE_SNAPSHOTS=1 cargo test`, review the new snapshots, and check
  `bench_lamp`. Renamed a style? Add the old name to `styles::ALIASES`.
- **A clock face**: `clock/<name>.rs` implementing `Face` (`name`,
  fixed-size `forms` most-preferred first, `draw` inside the form via
  `draw::Pen`), then add it to `clock::FACES`. The text fallback form is
  appended for you (`all_forms`).
- **A dock widget** (e.g. now-playing): one module implementing
  `dock::DockWidget` — `name` (also its config key `dock.<name>`),
  `default_place` (new widgets: `Place::Off`, so the default screen
  doesn't change), `forms(model, place)` (fixed sizes, most preferred
  first; `WidgetForm::fixed` / `::fill`, `variant` is yours to tell them
  apart in `draw`), `draw` (inside the rect only; colours via
  `model.theme`; `look.align` for forms narrower than the rect; spaces
  stay see-through, the ui adds the backing on the lava), `chip` (or
  `None`). Add it to `dock::WIDGETS` (order = stacking order and shrink
  priority) and give it a key: one `row(Widgets, "<k>", "<name>
  side/lava/off", &[(K('<k>'), A::Place("<name>"))])` in `KEYMAP` (a test
  checks every widget has one). Config, layout, help, toasts, chip
  ranking and the sweep tests pick it up; whatever state it shows lives
  on the `Model` (or its own module) and it reads it from there. Run
  `UPDATE_SNAPSHOTS=1 cargo test` only if a default changes.
- **A key**: add an `Action` variant (`ui/keymap.rs`), a `row(section,
  keys, label, &[(key, action)])` in `KEYMAP` (that's both dispatch and
  the help overlay), and its arm in `Model::global_action`
  (`app/model/actions.rs`); the match is exhaustive, so the compiler
  points at it. A bool setting is one line: `self.toggle(now, |s| &mut
  s.<field>, ["on toast", "off toast"])`. Status-bar hint? `chrome::HINTS`.

## Conventions & Patterns

- Simulation, clock, pomodoro, layout and the app `Model` are pure and
  testable; only `ui/` drawing and `app/mod.rs` touch the terminal.
- New render styles / clock faces / keys plug in via a trait or table +
  registry (see "Adding things"); no match-arms sprinkled across the
  codebase.
- Colours are decided only in `theme/`.
- Fixed simulation timestep, decoupled from render frame rate.
- Worker integration: create media/lyrics/Spotify workers before the frame
  loop; frame/input handlers only consume ready snapshots or send commands.
  No process spawning, HTTP, keyring, filesystem I/O, blocking receives or
  worker joins on that path. If a shared snapshot needs a lock, never hold
  it across I/O or backend calls; bound event draining per frame. Join only
  after drawing stops. Keep fake backends for deterministic model tests.
- Gate before handing off: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
