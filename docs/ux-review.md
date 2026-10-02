# First-time user review

Reviewed source baseline `7fbddf7` on macOS, using the release binary and
fresh settings files. LavaTUI makes the lamp easy to enjoy immediately, but
discovering controls and recovering from music setup problems still requires
terminal knowledge. The highest-impact changes are an in-app settings screen,
guided Spotify setup, and a visible way out of music controls.

## Top 10 findings

Each Bead contains reproduction steps, source/capture evidence, proposed
wording or behavior, acceptance criteria and an effort estimate. These are
proposals for review; no application source or design contract was changed.

| Rank | Bead | Finding and proposed fix |
|---|---|---|
| 1 | **lava-1xk.4 · P1** | Everyday settings require editing a file: timer lengths, bell, mouse, corner clock and cover size. Add an arrow-key/mouse settings sheet with named values and automatic saving. See `src/config/mod.rs:87` and `src/ui/keymap.rs:183`. |
| 2 | **lava-1xk.5 · P1** | `a`, `Shift+A`, `i` ends at “Spotify library needs a Client ID · see” at 80×24; the useful document path is dropped. Open a persistent “connect Spotify” guide with a Client ID field, setup steps and retry/back. Explain that desktop playback needs no library setup. See `src/app/model/library.rs:554`. |
| 3 | **lava-1xk.6 · P1** | Music controls silently own the keyboard after their toast fades in minimal/tiny windows. Keep “music controls · esc back” visible and teach that Space now controls music, while `q` first leaves this mode. See `src/ui/keymap.rs` and `src/app/model/music.rs:282`. |
| 4 | **lava-1xk.9 · P2** | Fresh 20×8 and 30×10 windows show no help or quit instructions. Add a dismissible first-run card; at tiny sizes prioritize “? help · q quit”. Teach saving, Shift shortcuts and the focus timer; defer the full card when space is insufficient. |
| 5 | **lava-1xk.7 · P2** | Narrow help changes meaning: “b d status bar · debug hud” becomes “b d status bar” at 20×8. Use one action per row and intentionally short labels. See `src/ui/help/mod.rs:137` and `src/ui/keymap.rs:275`. |
| 6 | **lava-1xk.10 · P2** | Tiny pickers show only “‹ outline ›”; users cannot discover save/cancel, and `?` does nothing there. Reserve “Enter save · Esc cancel” guidance, including bottom sheets in minimal mode. See `src/ui/picker.rs:132`. |
| 7 | **lava-1xk.8 · P2** | `m`, `Shift+C`, Down changes the selected clock face without a visible face preview; the corner clock remains text. Hiding the clock has the same problem. Add a temporary preview or a clear enlargement instruction, preserving placement on cancel. |
| 8 | **lava-1xk.11 · P2** | “␣ pomo”, unexplained capitals, “reseed wax”, “debug hud” and “sextant” assume specialist knowledge. Prefer “Space timer”, “Shift+S choose style”, “new wax pattern”, “performance info” and descriptive cover-quality names. Preserve stored IDs and bindings. |
| 9 | **lava-1xk.12 · P2** | The cover widget collapses permission-denied/not-running states to “nothing playing”, hiding the real recovery step. Reuse the player’s unavailable reason. Source-confirmed at `src/dock/cover.rs:210`; these OS states were not forced live. |
| 10 | **lava-1xk.13 · P2** | Help omits lamp/music mouse actions and always says Shift-drag, while the project documents Option-drag for Terminal/iTerm2. Add a short mouse section and accurate selection guidance. See `src/ui/keymap.rs:282`. |

Additional findings: **lava-1xk.14** explains lyric placement and which song
details go to lrclib.net; **lava-1xk.15** keeps music troubleshooting reachable
when a small layout drops its unavailable message. The existing **lava-4va**
already covers the help sheet’s height/scrolling issue, so it was not duplicated.
Internal terms such as dock, anchor, rank, chip and overlay mostly stay out of
current app labels; retain that separation.

## Larger proposals ranked by impact per effort

| Order | Proposal | Estimate | Impact |
|---|---|---|---|
| 1 | First-run welcome card — **lava-1xk.9** | 1–2 working days | Reaches every new user; teaches help, quit, saving and controls without permanent clutter. |
| 2 | In-app settings — **lava-1xk.4** | 4–6 working days | Removes file editing for common preferences, especially focus-timer and mouse settings. |
| 3 | Guided Spotify setup — **lava-1xk.5** | 3–5 working days | Replaces a developer-document dead end for playlists and likes. It must accurately explain the provider’s setup prerequisites. |

Plain-language key labels (**lava-1xk.11**, 1–2 days) are a useful companion
change. Estimates include responsive layouts, tests and visual review.
Welcome, settings and persistent mode hints require deliberate updates to
`docs/design.md`; they are not approval to add permanent visual clutter.

## Evidence and limits

Fifty-nine sized PTY runs (55 captures and four early-exit checks) covered
12×5, 20×8, 30×10, 50×16, 80×24, 120×36, 200×60 and
34×56. Captures cover launch, help/scrolling, style/clock/palette previews,
widget placement, `l`/`L`, player mode, no Client ID, lyrics opt-in, cover,
minimal mode, timer start/pause/skip/reset, mouse selection and wax heating,
save/restart and cancellation. Saved picker
choices returned on restart; canceled previews did not replace them. Mouse
selection changed the style. Separate quit checks verified `q`, help/picker
`q` then `q`, player `q` then `q`, and Ctrl+C cleanup.

The temporary harness, PNGs, terminal text and run summaries are in
`/tmp/lavatui-ux-review/`; they are local review artifacts, not release assets.
Representative PNGs were visually inspected. Spotify was already running:
playback/account controls and permissions were left unchanged. Missing-player
and permission-denied findings use the current code, existing fake-backend
tests and snapshots. Native text selection, OS permission prompts and browser
authorization need real-terminal/user QA. This is an expert walkthrough, not
a usability study with recruited beginners. No copyrighted cover was committed.

Release build, formatting and Clippy passed. The initial `cargo test` with
inherited `NO_COLOR=1` reported 500 passed, three failed and 16 ignored; all
three failures are cover tests that assume colour-capable defaults. Removing
that variable passed all 14 music-model tests. **lava-1xk.16** records the
test-fixture fix; no source fix was made here. The full retest with
`env -u NO_COLOR cargo test` passed: **503 passed, zero failed, 16 ignored**.
The inherited-environment failure remains a follow-up, not a fixed defect.
