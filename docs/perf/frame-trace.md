# Frame tracing and hitch investigation (lava-h52.1)

Measured 2026-10-01 on Apple M5, macOS arm64 in this workspace. This is application/pty timing, not terminal presentation latency. The strict acceptance criterion remains unproven: the final isolated p99 exceeds the 18.667 ms limit, and draw/wake gaps remain.

## Confirmed causes and changes

- Input redraws used to reset the pacer and then advance it again: a 34.828 ms post-input interval in the isolated baseline. Input now draws immediately without shifting the existing grid; its next isolated interval was 9.835 ms. Late frames skip expired grid slots instead of sleeping a new full period after the overrun.
- Config saves ran file I/O and fsync during tick: 25.326–45.296 ms in the concurrent baseline; 8.363 ms in the isolated matching-main baseline. The runtime now sends snapshots to a config worker with a one-slot queue and a superseding pending snapshot. Frame-side submission measured 4–23 microseconds in the matrix and 10 microseconds isolated. Store ownership preserves comments, backups, symlinks, hand edits and CLI overrides; quit drains pending saves and joins after drawing stops. Frozen mode checks outstanding results every ~100 ms.
- Terminal output was passed to stdout in thousands of formatting fragments (up to 69,504 successful Write calls for a full 300x90 solid frame in the matching-main baseline). A reusable Vec now collects a whole frame, including resize clears and bells, and sends it with write_all. BeginSynchronizedUpdate/EndSynchronizedUpdate wrap the batch. Output normally uses one successful Write call per frame; short/interrupted writes retry. These are Write calls, not counted OS syscalls.
- Cleanup sends EndSynchronizedUpdate and Show directly to stdout, even if a panic/error interrupts a buffered frame. Legacy non-ANSI Windows consoles retain passthrough output so WinAPI cursor/colour operations stay ordered with text.
- FixedStep is unchanged: elapsed real time times speed remains the simulation contract in design section 7. No arbitrary smoothing or discarded ordinary elapsed time was added.

## Five-minute concurrent matrix

Ten sized ptys ran together for 305 seconds, seed 7, truecolor, target 60 fps (adaptive quality enabled), default chrome, with a heat key at 3 seconds to exercise saving. First frame and immediate input frames are omitted from interval percentiles; the interval after input remains included. Units: milliseconds, each cell is p50 / p99 / max.

| Size / style | Before | After |
|---|---:|---:|
| 80x24-solid | 16.671 / 25.541 / 184.205 | 16.666 / 19.473 / 78.388 |
| 80x24-braille | 16.667 / 23.542 / 184.473 | 16.666 / 18.879 / 87.092 |
| 160x40-solid | 16.699 / 45.862 / 184.380 | 16.667 / 21.874 / 136.710 |
| 160x40-braille | 16.671 / 30.745 / 193.961 | 16.665 / 20.622 / 136.447 |
| 200x60-solid | 16.882 / 70.689 / 192.711 | 16.663 / 21.353 / 135.776 |
| 200x60-braille | 16.682 / 37.509 / 192.515 | 16.663 / 20.986 / 136.073 |
| 250x70-solid | 16.947 / 85.923 / 237.627 | 16.667 / 21.516 / 135.164 |
| 250x70-braille | 16.779 / 50.981 / 186.323 | 16.670 / 20.484 / 131.874 |
| 300x90-solid | 17.131 / 111.019 / 327.442 | 16.669 / 21.285 / 135.779 |
| 300x90-braille | 16.995 / 64.438 / 217.461 | 16.657 / 20.163 / 131.117 |

Adaptive quality temporarily reduced six baseline cases to 30 fps under load; all after cases stayed at 60 fps. Spike thresholds use each frame's recorded target rate. This is a stress comparison, not a controlled estimate of improvement: the initial baseline is 7eb78b0 with instrumentation; the after matrix includes a536ef8 (new dock and blob shapes), and both overlap builds/other pty work. At ~277 seconds in the after matrix, large residual/writer stalls appeared across many independent ptys together. That is evidence of shared host/I/O interference, not proof of its precise cause. Do not exclude all such spikes as terminal stalls or assert acceptance from this table.

## Isolated matching-main comparison

A single 300x90 solid pty ran for 305 seconds before and after, with identical seed, size, style, config and key script. Baseline was an instrumented scratch copy of dfd56bd with original pacing, passthrough stdout and inline config saving restored. The final candidate includes dfd56bd plus these fixes; no render/sim/theme changes were authored here. Background host load was uncontrolled (final 1-minute load average 5.31 at start, 6.16 at end).

| Run | Frames | p50 | p99 | Max | >2-period gaps |
|---|---:|---:|---:|---:|---:|
| Before | 18,275 | 16.664 | 20.702 | 52.287 | 3 |
| After | 18,270 | 16.664 | 20.671 | 56.701 | 6 |

The isolated timing threshold is not met. Four final spikes occur mostly in input wait/wake (19.7–34.7 ms deadline misses), and two spend 20.871/23.335 ms in ui::draw. Trace durations are wall time, so these draw spans cannot distinguish CPU work from a thread being descheduled. Those drawing hot loops belong to lava-h52.2. Native Ghostty capture with the integrated optimizer is still needed; pty completion cannot establish when Ghostty presents a frame.

## Combined build (d5dfcaa)

Main was merged again after lava-h52.2 landed. Eight ptys ran together for 305 seconds, with the same seed/colour/key settings. All stayed at 60 fps. This adds compute optimizations to the candidate, so compare it as an integrated build rather than attributing every change to the frame-loop patch.

| Size / style | p50 / p99 / max (ms) | >2-period gaps |
|---|---:|---:|
| 160x40-braille | 16.674 / 21.374 / 84.281 | 11 |
| 160x40-solid | 16.672 / 22.021 / 84.761 | 9 |
| 250x70-braille | 16.677 / 21.263 / 83.610 | 12 |
| 250x70-solid | 16.668 / 21.609 / 84.564 | 11 |
| 300x90-braille | 16.671 / 21.132 / 81.068 | 12 |
| 300x90-solid | 16.690 / 21.392 / 83.613 | 11 |
| 80x24-braille | 16.663 / 18.678 / 45.791 | 6 |
| 80x24-solid | 16.665 / 20.881 / 68.913 | 7 |

The combined build still does not establish the acceptance threshold. Retest the native Ghostty output/presentation path under controlled host load; all gaps are retained in the evidence, not discarded. Current bead remains open for that acceptance work.

## Evidence and reproduction

All >2-period gaps from the five series, with measured component durations and an annotated location, are in [frame-spikes.csv](frame-spikes.csv). Annotations identify measured locations, not proven scheduler/terminal causality. Compact interval summaries are in [frame-trace-summary.json](frame-trace-summary.json). Raw CSV traces and complete harness summaries remain under /tmp/lavatui-trace-{before-runs,after-runs,isolated-before,isolated-after,integrated-runs}.

```sh
cargo build --release
# Concurrent matrix (use one case at a time for isolated measurements):
python3 tools/trace_frames.py --output /tmp/lavatui-traces --seconds 305 --sizes 80x24 160x40 250x70 --change
python3 tools/trace_frames.py --output /tmp/lavatui-large --seconds 305 --sizes 300x90 --styles solid --change
# Native terminal: quit normally to flush trace rows to disk.
target/release/lavatui --trace /tmp/ghostty-frames.csv --seed 7
# Equivalent opt-in environment variable: LAVATUI_TRACE=/tmp/ghostty-frames.csv
```

Trace columns: wait start/end and frame start/end relative to capture origin; completion interval; input flag; tick and draw durations; residual terminal work (diff/encoding, size check, buffer swap, local clock/bookkeeping) excluding measured tick/draw/I/O; write and flush durations; bytes and successful Write calls; simulation step count, fixed dt and clamped/scaled time fed into the accumulator; frame-side save duration; lateness at frame start; target fps. Times ending in _us are microseconds; sim dt/feed are seconds. Trace rows remain in memory and are written on normal/error exit so tracing performs no frame-thread file I/O. A panic does not write accumulated trace rows.

## Validation

cargo fmt --check; cargo clippy --all-targets -- -D warnings; cargo test (385 passed, 11 ignored after compute integration); release build. Regression tests cover early/late fake-clock pacing, immediate input grid preservation, real ratatui clear/diff/flush batching, partial/interrupted writes, legacy output fallback, direct cursor cleanup, worker save coalescing/hand edits and final CLI-override-safe save. Normal and deliberate-panic sized-pty smoke checks verify synchronized framing, cursor restoration and alternate-screen exit. Read-only review findings were addressed. No visual layout/style changes require new screenshots.


## Deadline wake follow-up (main 8a977bc)

The focused, animated UI requests macOS USER_INTERACTIVE QoS through libc. The original hint is restored when the lamp becomes idle or unfocused, and on leaving the loop (an unspecified CLI class falls back to DEFAULT, since the setter cannot request UNSPECIFIED). Workers reset to DEFAULT at entry, including workers created after startup; they do not retain the UI priority or pass it to osascript. Failure to set a scheduling hint leaves the app usable.

The input wait polls until 1.5 ms before the deadline. It then checks input between sleeps of at most 100 µs and uses CPU relaxation (`spin_loop`) for the final at-most-200 µs window. This preserves immediate input and bounds active waiting to 200 µs per scheduled wait. A scheduling probe under load found OS `yield_now` itself could return about 10 ms late, even at interactive QoS (1000 repetitions: 100 µs yield window p50 138 µs, p99 2282 µs, max 10027 µs); 100 µs sleeps were p50 129 µs, p99 176 µs, max 194 µs. An initial yield-based candidate is retained in the evidence, and the final implementation avoids OS yield in the deadline window. Idle/frozen and unfocused waits retain ordinary blocking input polling. A scheduler can still delay any wake; this is a scheduling hint and a bounded precision window, not a real-time guarantee.

`wait_cpu_us` is a trace-only macOS thread CPU measurement around the complete input wait, including input dispatch. Zero on unsupported platforms or clock failure means unavailable. The summary prints this CPU cost as a percentage of one core. It is an upper bound on the additional precise-wait cost, not a subtraction of baseline polling CPU. Total process CPU comes separately from the pty harness's wait4 usage.

The fake clock covers deadline completion, a late blocking poll and sleeps, input in both wait phases, zero-duration queue drains, and oversleep past the deadline. A real macOS thread test checks priority restoration and worker priority reset. The merged music lifecycle test now explicitly selects truecolor so its cover assertion is independent of NO_COLOR in the test runner.

## Isolated QoS + precise-wake comparison

Each case ran alone for 185 seconds, before then after, on matching main 8a977bc, seed 7, truecolor, default chrome and music off, target 60 fps. The final group started after waiting six minutes for the host to settle; load never reached the requested low-load goal (1-minute load was 19.25 at final group start, and had peaked above 67 while other builds/tests ran). No local builds overlapped the final captures. These are three-minute diagnostic captures, not the bead's five-minute acceptance run or native Ghostty presentation measurements.

| Size / style | Before p50 / p99 / max (ms) | Final p50 / p99 / max (ms) | Before → final >2-period gaps |
|---|---:|---:|---:|
| 300x90-solid | 16.679 / 20.577 / 46.545 | 16.664 / 18.953 / 31.923 | 6 → 0 |
| 300x90-braille | 16.669 / 21.437 / 77.130 | 16.666 / 18.963 / 29.862 | 32 → 0 |
| 200x60-solid | 16.656 / 19.034 / 48.007 | 16.664 / 18.098 / 33.585 | 1 → 1 |
| 200x60-braille | 16.663 / 19.173 / 54.668 | 16.664 / 18.129 / 32.079 | 3 → 0 |

| Size / style | Total process CPU before → final (% of one core) | Final complete input-wait CPU (% of one core) | Median wake lateness before → final (µs) |
|---|---:|---:|---:|
| 300x90-solid | 23.92 → 15.57 | 0.50 | 1010 → 8 |
| 300x90-braille | 21.87 → 17.34 | 0.51 | 1010 → 8 |
| 200x60-solid | 11.41 → 7.95 | 0.52 | 1011 → 8 |
| 200x60-braille | 11.98 → 8.36 | 0.51 | 1012 → 8 |

Complete wait CPU includes polling and dispatch, so it bounds the extra precise-wake cost rather than measuring an incremental subtraction. The final relaxation window can consume about 1.2% of one core at 60 fps (200 µs × 60), plus input-poll/check overhead. Process CPU includes startup and frame work; uncontrolled host load prevents a clean causal CPU comparison.

| Size / style | Before 1-minute load start → end | Final 1-minute load start → end |
|---|---:|---:|
| 300x90-solid | 12.06 → 4.69 | 19.25 → 16.69 |
| 300x90-braille | 4.69 → 8.13 | 16.69 → 17.98 |
| 200x60-solid | 8.13 → 9.33 | 17.98 → 9.83 |
| 200x60-braille | 9.33 → 9.12 | 9.83 → 16.87 |

Final gap locations (wall-time stages, not proven scheduler causes):

- 300x90-solid: no >2-period gaps.
- 300x90-braille: no >2-period gaps.
- 200x60-solid: one stdout-write-stage gap, frame 8994: interval 33.585 ms, write 17.070 ms, wake lateness 9 µs.
- 200x60-braille: no >2-period gaps.

All new gaps remain in [frame-spikes.csv](frame-spikes.csv); no terminal/scheduler exclusions were applied. Full compact metrics, CPU and load metadata are in [frame-trace-summary.json](frame-trace-summary.json), under `qos-before`, `qos-yield-candidate` and `qos-spin-after`. Raw traces remain in `/tmp/lavatui-qos-before-{size}-{style}`, `/tmp/lavatui-qos-after-{size}-{style}` (the rejected yield trial, two completed 300×90 runs), and `/tmp/lavatui-qos-spin-{size}-{style}` (final). The third yield-trial case was deliberately interrupted after the primitive was rejected and is not included as a completed capture.

Validation: cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test (410 passed, 11 ignored), release build, old/new-schema summary output and CPU arithmetic checks, and read-only review. The bead remains open for strict timing acceptance and native-terminal verification; these diagnostics do not justify claiming every hitch eliminated.

## Integrated remeasurement (main 6250200)

Main advanced during capture to include Dock v2, synced lyrics, and the cheaper/platform media backends. It was merged into this branch as c560005; the dependency conflict retains macOS libc, Linux zbus and Windows APIs. The wake/QoS changes survived the merge. The osascript process and its reader are started lazily from Backend::exchange on the DEFAULT-priority media worker; the UI does not launch or wait on that process.

Four more isolated 185-second cases ran with the same seed, colour, target rate and scratch defaults (music/lyrics off). This verifies the integrated build; it is separate from the matching-8a977bc before/after comparison above. All cases stayed at 60 fps.

| Size / style | p50 / p99 / max (ms) | >2-period gaps | Wait CPU (% of one core) | Total process CPU (% of one core) | 1-minute load start → end |
|---|---:|---:|---:|---:|---:|
| 300x90-solid | 16.664 / 18.903 / 27.046 | 0 | 0.52 | 14.78 | 10.95 → 8.51 |
| 300x90-braille | 16.660 / 19.367 / 44.420 | 1 | 0.56 | 17.66 | 8.51 → 7.03 |
| 200x60-solid | 16.669 / 18.370 / 28.027 | 0 | 0.59 | 9.74 | 7.03 → 4.02 |
| 200x60-braille | 16.653 / 18.529 / 28.341 | 0 | 0.61 | 11.61 | 4.02 → 3.47 |

There was one gap: 300×90 braille frame 10131, interval 44.420 ms; stdout write 29.818 ms, tick 0.023 ms, draw 1.593 ms, residual diff/encoding 0.462 ms, wake lateness 13 µs. It remains in the CSV; the trace identifies the output stage but cannot establish whether the writer blocked or was descheduled there. All other integrated cases had no >2-period gaps. Complete input-wait CPU is 0.52–0.61% of one core, bounding the extra wake cost. Median wake lateness is 9–10 µs. Native Ghostty capture remains needed for the actual renderer/presentation workload, especially with the user's usual widgets enabled.

Integrated gates: cargo fmt --check, cargo clippy --all-targets -- -D warnings, cargo test (445 passed, 12 ignored), release build. Review verified merge retention and worker priority inheritance. Normal and deliberate-panic sized-pty smoke checks cover synchronized framing, cursor restoration and leaving the alternate screen. No author changes were made in sim/render/theme hot loops; changes there came from main. No push or remote sync.

## Native Ghostty capture

Run this one-liner from the repository root in native Ghostty, at the size where the hitch occurs:

```sh
cargo run --release -- --trace /tmp/lavatui-ghostty.csv --fps 60 --frames 18000 --seed 7 && python3 tools/trace_frames.py --summarize /tmp/lavatui-ghostty.csv
```

It exits after 18,000 frames (about five minutes at focused 60 fps); `q` ends it early and still writes the trace. Keep the window focused to measure 60 fps. The summary prints p50/p99/max frame intervals, wake lateness, wait CPU, and every gap exceeding two frame periods with its measured stage. Reproduce with the usual widgets/settings so the real workload is captured. Save output alongside the terminal size and host load (`uptime`). The trace measures app frame completion, not Ghostty's actual presentation time; synchronized output lets Ghostty present the completed batch together. Existing captures can be summarized without a TTY using `python3 tools/trace_frames.py --summarize /tmp/lavatui-ghostty.csv`.
