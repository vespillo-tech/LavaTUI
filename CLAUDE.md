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
(outline, heatmap, ASCII, dither, braille, halftone, …), with an optional
very basic lighting/glow pass. Ships with a selectable-style clock and a
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

## Architecture Overview

- `main.rs`   — parse CLI, `ratatui::try_init` (raw mode, alt screen, panic
                hook that restores the terminal), run app, `ratatui::restore`.
- `cli.rs`    — clap derive flags (`-m/--minimal`, `--fps`, `--style`,
                `--palette`, `--color`, `--seed`, `--config`, hidden
                `--frames`) → `config::Session` (session-only overrides).
- `config/`   — `Settings`: the persisted TOML surface of design §9 (serde,
                every field defaulted, `sanitized()` clamps). `Session` layers
                CLI flags on top; `to_persist` puts the file's values back for
                fields a flag still holds, so flags are never written back.
                `store.rs`: XDG path (`$XDG_CONFIG_HOME/lavatui/config.toml` or
                the `directories` config dir), load (missing → defaults,
                corrupt → defaults + toast message, backed up to `.bak` on
                first save), atomic save.
- `app/`      — `mod.rs` is the loop only: poll input until the frame
                deadline → `Model::update(action)` (any input draws at once;
                queued events are drained first) → `Model::tick(now, area,
                local_time)` → `ui::draw`. It owns the terminal, reads local
                time (jiff) and cell aspect (`window_size` pixels), rings the
                bell, enables focus reports (+ mouse capture if
                `input.mouse`). `model.rs`: all state (settings, world,
                style/theme/face, pomodoro, overlay, toast, layout) and all
                behaviour: `update` applies one `Action` (overlay keys first;
                under an overlay only quit/resize/focus get through),
                `tick` advances pomodoro/toasts/flash/eased speed/sim steps,
                recomputes the layout, matches the sim's `Shape` to the frame,
                and does the debounced (1 s) save. Fps: 10 unfocused, 2 frozen.
- `timing.rs` — pure loop timing: `FixedStep` (accumulator, max 8 steps per
                frame, `alpha()` for interpolation), `FramePacer` (fixed-grid
                frame deadlines, resyncs when late), `FpsMeter` (EMA).
- `sim/`      — wax simulation (pure, seeded, deterministic). `World::new(seed,
                aspect, Shape)` + `step(dt)` at the fixed `dt` (`SIM_HZ = 120`).
                World units: height 1, width = visual aspect, x centred on 0.
                Pool on the heater buds blobs; heat/buoyancy/drag/cohesion,
                merge + split, melt back into the pool; wax area conserved.
                `field.rs`: `Field::prepare(&world, alpha)` once per frame,
                then `fill(&mut [Sample], cols, rows)` / `sample(u, v)`
                (v down; density `>= SURFACE` is wax). `controls.rs`: heat,
                reseed, heat pulse, `SimSpeed`, `set_shape` (glass ↔ bleed:
                melts the wax into the pool and re-buds, like reseed). Model
                notes and all tuning constants are at the top of `sim/mod.rs`.
- `theme/`    — palettes + colour depth: the only place colours are decided.
                `Palette` (9 `Role`s × 8 palettes from design §5.2, hex/256/16),
                `ColorDepth::detect()` (NO_COLOR → COLORTERM → TERM, §5.3),
                `Theme::new(palette, depth)`. Styles ask for `Ink::Role(r)`,
                `Ink::Wax(t)` (cool→mid→hot) or `Ink::Heat(t)` (liquid→hot) via
                `theme.color(ink)` / `theme.paint(ink).mix(..).scale(..).color()`;
                16/none never blend (dominant side wins), 256 snaps to xterm.
- `render/`   — render pipeline. `LampView { field, style, theme, time,
                lighting }` is a `StatefulWidget` (state `LampState` = reused
                scratch buffers); it samples the field at the style's `Grid`
                (half-block 1×2, braille 2×4, …; >400k samples → coarse fill +
                bilinear upsample), builds the glass mask, runs the optional
                lighting pass, then calls `Style::draw(&Canvas, area, buf)`.
                Styles live one per file in `render/styles/`, registered in
                `styles::ALL` (`StyleId` cycles/looks up). Shared helpers:
                `coverage` (quantised AA edge), `wax_heat`, `bayer`,
                `cell::{half_block, braille}`. Snapshots: `render/snapshots/`
                (`UPDATE_SNAPSHOTS=1 cargo test` to rewrite, then review).
- `light/`    — `Lighting` trait: the seam for the glow pass (lava-5ak). Fills a
                per-sample brightness buffer that styles read via `Canvas::light`.
                `ui::draw` passes `lighting: None` today; the `l` key and
                `lamp.lighting` setting are already wired for it.
- `clock/`    — clock faces (`Face` trait + `FACES` registry: blocks, segment,
                analog, binary, words, text; each lists fixed-size `Form`s and
                `fit()` picks the largest that fits) and the pomodoro state
                machine (`Pomodoro`, pure, `Instant` passed in) +
                `PomodoroWidget`. Faces leave spaces transparent.
- `ui/`       — the only terminal-facing code. `layout.rs`: the pure
                `layout(area, &LayoutInput) -> Layout` of design §1 (frame
                glass/bleed, margins, right/bottom panel, chip, status row,
                toast row; hide order date → margins → face size → glass →
                panel). `keymap.rs`: the single `KEYMAP` table that drives
                both dispatch (`action_for(event, InputMode)`) and the help
                overlay. `mod.rs` draws back to front; `glass.rs` (cap/base in
                shaded metal with half-cell edges, `▕ ▏` walls in 16/none),
                `panel.rs` (face + date + pomodoro, chip), `chrome.rs` (status
                bar + hint fitting, HUD, toasts), `help.rs`, `picker.rs`.
                `tests.rs`: size sweep 1×1..300×100 × 8 setting variants
                (no overlap/overflow, lamp always there) + mockup-size checks
                + layout snapshots in `ui/snapshots/`.

crossterm is used via ratatui's re-export (`ratatui::crossterm`) so the two
never drift apart; there is no direct crossterm dependency.

## Conventions & Patterns

- Simulation, clock, pomodoro, layout and the app `Model` are pure and
  testable; only `ui/` drawing and `app/mod.rs` touch the terminal.
- New render styles / clock faces plug in via a trait + registry; no
  match-arms sprinkled across the codebase.
- Fixed simulation timestep, decoupled from render frame rate.
- Gate before handing off: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.
