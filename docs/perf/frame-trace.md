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
