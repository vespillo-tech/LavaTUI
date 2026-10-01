# Compute profiling (lava-h52.2)

Baseline: `dfd56bd` (main, including the widget dock, larger/varied blobs,
media/lyrics/Spotify modules and opaque synthwave wax). The earlier diagnostic
sweep used `a536ef8`; synthwave changed between those sweeps, so final comparisons
use the same `dfd56bd` baseline on both sides. No simulation tuning, app loop,
frame pacing or output protocol changes belong to this patch.

## Method

Apple M5, release build. Run tests alone with `--test-threads=1`.
`bench_compute` measures 1,800 evolving frames (30 simulated seconds), two
120 Hz steps per frame, seed 7 after 1,200 warmup steps. Sizes: 80×24, 160×40,
250×70 and 300×90; all nine styles, truecolor and ANSI256. It separates step,
field preparation, buffer reset, field fill, style draw, ordered dither and the
streaming ratatui buffer diff. Draw includes the dithering theme copy. Initial
sample/buffer allocation is outside stage timing. Simulation vector growth during resize and topology bursts are investigated
separately by `bench_step_outliers`; renderer scratch growth on resize is outside
these steady-size stage timings.

Stage timings use monotonic wall time; total also uses the macOS thread CPU
clock, which excludes descheduling. Clock calls and instrumentation add small
measurement overhead; the benchmark is diagnostic, not the production loop.
The output fingerprint hashes every field sample's density/temperature bits and
every buffer cell (glyph, colours, attributes). No snapshots were regenerated
by this patch. Synthwave snapshot changes came from the upstream merge.

Colour caches are global and persist between rows. `first_dither_us` means the
first measured frame, not necessarily a cold cache. To measure a particular
style's cold cache, launch the test in a fresh process with `LAVA_BENCH_SIZE`,
`LAVA_BENCH_DEPTH`, and `LAVA_BENCH_STYLE`. `LAVA_BENCH_FRAMES` changes duration.
`bench_xterm_cold` bypasses the cache and searches all 262,144 RGB buckets.
Its fingerprint checks the packed near/far/mix-level answers against the baseline;
a regular differential test also checks the original stable-ranking algorithm
on 8,192 representative buckets.

```sh
cargo test --release bench_compute -- --ignored --nocapture --test-threads=1
cargo test --release bench_step_outliers -- --ignored --nocapture --test-threads=1
cargo test --release bench_xterm_cold -- --ignored --nocapture --test-threads=1
cargo test --release bench_fill bench_lamp -- --ignored --nocapture --test-threads=1
```

## Profile and changes

Installed samply 0.13.1 with `cargo install samply --locked`. Recorded sized,
drained PTYs at 4 kHz with `samply record --save-only --unstable-presymbolicate`.
The initial 250×70 chrome/256 run (900 frames, minimal mode) sampled 955 ms of
thread CPU over 15.49 s. Inclusive costs: chrome draw 43.8%, 256 resolver 13.5%,
field fill 2.0%, simulation step under 0.1%. Buffer coordinate indexing alone
was 4.6% self CPU. This is a sampled profile, not an exact per-stage timer.

1. Shared canvas iteration hoists grid ratios and liquid colour out of the
   cell loop, indexes once per buffer row and traverses its slice. It preserves
   offset rectangles and the containing buffer's stride. Matrix retains its
   column-major traversal.
2. The 256 resolver similarly indexes once per row and keeps Bayer coordinates
   anchored to the lamp. No colour or glyph rule changes.
3. Cold xterm pair searches use fixed stack storage instead of a growing Vec
   and an allocating stable sort. Explicit candidate-index tie-breaking
   preserves the old stable ordering. Single-index answers skip sorting.
4. OKLab chroma is computed once per colour rather than repeatedly by hue gates
   and pair-distance comparisons. The original floating-point expression is
   preserved. The cache remains fixed-size atomics, not a HashMap.

The sim already reuses its acceleration buffer and Field snapshot storage;
LampState reuses samples/coarse samples. Kernels are resolution-dependent, so
cross-frame kernel caching would still invalidate each animated frame. Sampling
already uses per-kernel influence boxes. No numerical or simulation changes were
justified by this profile. Buffer diff already streams in production; the
older `bench_lamp` uses `diff()` and allocates a Vec, so `bench_compute` uses
`diff_iter()` to match production.

The machine was heavily contended during the first diagnostic sweep (load
average peaked at 52). Wall-time stalls affected unrelated stages, including
buffer reset and two sub-microsecond sim steps. Thread CPU totals distinguish
these delays from compute. Absolute wall maxima are reported without deleting
outliers; they must not be read as proof of an algorithmic spike or a guarantee
of smooth presentation under host contention.

## Simulation stress

`bench_step_outliers` runs five heat-5 worlds for 72,000 steps each, including
wide/narrow wall changes every 60 simulated seconds and periodic heater pulses.
The spawning cap is 40; forced splits during resize can temporarily exceed it.
The widest case reached 62 blobs, so the pairwise stages were exercised above
the normal target, not only at the seven-to-eight-blob default workload.

| Aspect | Step mean / p99 / max (µs) | Event / quiet step max (µs) | Max blobs | Bud / merge / split / melt events |
|---|---|---|---|---|
| 0.400 | 0.59 / 1.33 / 37.75 | 1.79 / 37.75 | 16 | 121 / 103 / 5 / 44 |
| 1.667 | 0.86 / 2.71 / 15.62 | 5.21 / 15.62 | 25 | 76 / 77 / 70 / 90 |
| 2.000 | 0.99 / 3.04 / 43.83 | 5.04 / 43.83 | 23 | 95 / 77 / 59 / 101 |
| 1.786 | 0.94 / 3.46 / 38.71 | 4.58 / 38.71 | 25 | 74 / 78 / 60 / 86 |
| 10.000 | 4.00 / 11.75 / 88.04 | 17.21 / 88.04 | 62 | 284 / 171 / 157 / 285 |

Event steps were faster than the worst quiet steps. Pairwise work, merging,
splitting and temporary vector growth did not approach the 16.67 ms budget in
this stress run. An O(n²) broad phase or SIMD/numeric rewrite would add risk
without addressing a measured hitch here.

## Current-main PTY profile

300×90 topo/ANSI256 with clock and pomodoro overlays enabled, seed 7, 900
frames at the default 60 fps. The PTY master was continuously drained; terminal
emulation/GPU presentation is outside this profile. Baseline completed in
16.55 s and the optimized build in 15.45 s. Output byte totals are not expected
to match because these real-time runs advance at different wall-clock instants.
The deterministic benchmark below is the byte-identity comparison.

| Sampled CPU (ms, inclusive except index lookup) | Baseline | Optimized |
|---|---:|---:|
| Total sampled thread CPU | 3904 | 3690 |
| Topo draw | 1816 | 1687 |
| Dither resolution | 298 | 185 |
| Buffer index lookup (self) | 154 | 20 |
| Field fill | 907 | 927 |
| Sim step | 1.7 | 1.5 |

Index lookup fell 87% and dither resolution fell 38% in this sampled comparison.
Overall sampled CPU fell 5.5%; background contention prevents treating the exact
percentage as a portable speedup. Field arithmetic and simulation are unchanged.
The existing `bench_fill` and `bench_lamp` also ran under samply: all benchmarks
passed. Final lamp means ranged from 22–180 µs at 80×24 and 116–751 µs at 200×60
across truecolor/256/16. `bench_fill` measured 60 µs at 200×120 samples, 134 µs
at 300×200, and 10 µs at 80×48. These use a different aspect/workload than the
full-window tables below and are not substitute before/after comparisons.

## Cold matcher

Exhaustive uncached search across all 262,144 buckets, one fresh process per
build. Both builds produced packed-pair fingerprint `92ca1e2a43cb68c6`.

| Search time (µs) | Baseline | Optimized |
|---|---:|---:|
| Mean | 2.77 | 1.41 |
| p99 | 8.83 | 7.25 |
| Max (wall) | 466.79 | 114.04 |

Earlier 8,192-bucket diagnostic timings isolated the stack-storage change:
2.87 → 2.61 µs mean and 72.17 → 57.62 µs max. The later no-sort and chroma
changes were then applied sequentially. The no-sort run was contention-affected
(4.41 µs mean); it does not establish an isolated speedup. The combined optimized
8,192-bucket run was 1.41 µs mean / 41.00 µs max, with identical output. The full
exhaustive comparison above is the final evidence for the combined matcher
changes, not a claim that each individual subchange has that same speedup.

## Final stage timings

The following paired sweeps use `dfd56bd` on both sides. Every one of the 72
field/cell fingerprints matched over all 1,800 frames. Columns show **baseline
mean/max → optimized mean/max**, in µs. They include every wall-time outlier;
CPU total excludes descheduling. The raw stages remain wall time, so a slow
host can inflate stage maxima independently of an optimization. Initial full
sweeps were repeated; the 300×90/256 tail showed host slowdown and was rerun
in alternating fresh processes (see the separate comparison after these tables).

### 80x24

| Depth / style | step2 | prepare | reset | fill | draw | dither | diff | wall total | CPU total |
|---|---|---|---|---|---|---|---|---|---|
| truecolor / solid | 0.91/6.67 → 1.15/9.21 | 0.17/37.38 → 0.21/14.12 | 1.59/14.54 → 1.94/13.42 | 11.91/62.33 → 14.70/240.96 | 34.26/169.54 → 36.63/118.96 | 0.02/6.04 → 0.02/0.08 | 22.48/72.38 → 25.66/66.29 | 71.51/350.58 → 80.50/363.04 | 71.65/351.42 → 80.28/199.33 |
| truecolor / outline | 0.87/6.33 → 1.12/10.71 | 0.14/1.00 → 0.23/66.42 | 1.60/13.00 → 1.93/25.08 | 41.43/63.00 → 54.12/4576.79 | 36.80/71.79 → 35.14/745.58 | 0.02/0.04 → 0.02/0.12 | 13.13/23.33 → 16.09/1149.67 | 94.15/131.67 → 108.83/4629.08 | 94.27/128.29 → 103.19/211.79 |
| truecolor / ascii | 0.84/2.00 → 1.05/19.25 | 0.13/1.04 → 0.17/1.46 | 1.51/2.00 → 1.82/10.46 | 11.15/48.21 → 13.43/72.29 | 14.95/23.96 → 15.28/236.50 | 0.02/0.04 → 0.02/6.83 | 11.29/19.17 → 13.41/34.75 | 40.06/77.25 → 45.37/333.29 | 40.24/60.88 → 45.22/103.83 |
| truecolor / braille | 0.89/45.04 → 1.01/15.38 | 0.14/0.96 → 0.16/1.17 | 1.61/6.50 → 1.79/8.67 | 41.45/79.58 → 45.89/113.75 | 35.20/46.29 → 33.39/95.88 | 0.02/0.04 → 0.02/0.12 | 15.25/25.33 → 16.85/51.38 | 94.72/139.79 → 99.29/241.25 | 94.86/117.83 → 99.14/241.67 |
| truecolor / halftone | 0.92/32.21 → 0.88/2.29 | 0.14/1.29 → 0.14/1.21 | 1.59/16.79 → 1.64/8.29 | 11.69/346.42 → 11.52/41.08 | 15.08/46.12 → 9.89/37.71 | 0.02/0.08 → 0.02/0.04 | 15.46/59.62 → 15.40/38.75 | 45.06/383.54 → 39.67/83.25 | 44.86/112.62 → 39.80/64.54 |
| truecolor / synthwave | 0.92/14.50 → 0.91/7.96 | 0.15/4.62 → 0.15/15.88 | 1.64/19.50 → 1.65/16.58 | 11.97/42.04 → 11.71/56.58 | 66.37/181.88 → 54.73/251.00 | 0.02/0.08 → 0.02/0.04 | 22.62/79.21 → 21.57/58.88 | 103.88/273.21 → 90.91/287.67 | 103.53/261.79 → 90.74/176.21 |
| truecolor / matrix | 1.02/24.04 → 0.89/2.04 | 0.17/10.54 → 0.14/1.12 | 1.85/46.25 → 1.62/8.08 | 6.89/87.21 → 6.17/33.75 | 19.57/98.96 → 18.60/51.29 | 0.02/0.12 → 0.02/0.04 | 12.59/51.79 → 12.26/52.42 | 42.28/146.67 → 39.86/79.08 | 41.16/122.54 → 39.94/63.00 |
| truecolor / topo | 1.30/19.38 → 1.14/163.21 | 0.22/6.79 → 0.17/10.12 | 2.08/94.62 → 1.66/10.79 | 52.39/396.58 → 43.34/385.88 | 117.08/604.12 → 93.85/656.79 | 0.02/0.17 → 0.02/0.17 | 18.30/133.21 → 16.11/784.92 | 191.61/713.75 → 156.45/1541.00 | 185.31/340.92 → 154.44/392.88 |
| truecolor / chrome | 0.90/36.08 → 0.84/2.29 | 0.14/1.00 → 0.14/3.58 | 1.57/13.75 → 1.55/5.58 | 11.55/58.00 → 10.95/31.88 | 53.86/109.67 → 51.52/88.17 | 0.02/0.08 → 0.02/0.04 | 21.86/48.50 → 21.04/35.62 | 90.05/144.96 → 86.22/125.79 | 89.98/133.04 → 86.33/108.88 |
| 256 / solid | 0.92/35.12 → 0.83/2.17 | 0.14/1.04 → 0.13/0.88 | 1.64/27.38 → 1.53/1.75 | 11.97/49.08 → 10.76/22.29 | 29.43/61.92 → 23.13/38.25 | 9.37/352.96 → 5.50/223.25 | 21.88/75.67 → 18.91/33.67 | 75.52/420.71 → 60.94/285.21 | 75.41/421.00 → 61.11/285.38 |
| 256 / outline | 0.88/20.75 → 0.86/2.83 | 0.14/2.50 → 0.14/1.83 | 1.58/3.62 → 1.56/3.42 | 40.73/77.96 → 39.95/107.71 | 36.03/75.42 → 28.34/70.88 | 6.78/26.21 → 4.01/10.71 | 12.60/43.79 → 12.50/61.79 | 98.91/141.67 → 87.52/154.12 | 98.86/132.83 → 87.58/117.46 |
| 256 / ascii | 1.01/18.46 → 0.83/2.58 | 0.17/6.04 → 0.13/0.92 | 1.80/37.62 → 1.49/1.88 | 12.90/86.00 → 10.86/22.96 | 16.25/147.08 → 12.08/21.83 | 8.07/153.46 → 4.34/54.71 | 11.74/49.58 → 10.44/16.50 | 52.17/351.83 → 40.34/105.42 | 51.00/137.38 → 40.51/105.62 |
| 256 / braille | 1.03/19.71 → 0.84/2.83 | 0.17/5.42 → 0.13/0.83 | 1.83/19.33 → 1.55/10.92 | 45.82/114.04 → 39.42/52.88 | 38.54/89.67 → 28.67/63.62 | 7.80/58.88 → 4.26/25.04 | 16.31/48.96 → 14.70/28.25 | 111.67/236.71 → 89.73/124.79 | 110.71/237.33 → 89.88/110.46 |
| 256 / halftone | 0.95/12.29 → 0.82/8.17 | 0.15/1.25 → 0.13/0.75 | 1.71/19.21 → 1.51/2.71 | 12.18/50.29 → 10.67/21.08 | 15.32/54.92 → 8.93/18.42 | 7.51/38.92 → 4.08/13.79 | 15.27/62.21 → 13.81/24.54 | 53.27/128.54 → 40.11/54.29 | 53.12/128.88 → 40.28/53.17 |
| 256 / synthwave | 1.27/27.50 → 0.83/2.25 | 0.22/1.54 → 0.13/0.71 | 1.90/46.04 → 1.52/1.92 | 14.85/203.12 → 10.82/27.92 | 82.53/2278.54 → 47.44/85.42 | 13.98/434.83 → 7.16/304.83 | 24.91/627.71 → 18.72/29.62 | 139.89/2368.92 → 86.79/399.38 | 134.72/547.58 → 86.94/399.54 |
| 256 / matrix | 1.21/10.96 → 0.83/2.08 | 0.21/7.04 → 0.13/0.88 | 1.91/21.29 → 1.49/1.83 | 8.05/128.62 → 5.72/36.25 | 22.10/118.83 → 16.56/33.04 | 9.23/155.08 → 4.18/75.29 | 13.29/221.58 → 10.82/18.12 | 56.19/311.38 → 39.89/123.17 | 55.74/207.67 → 40.07/123.38 |
| 256 / topo | 1.15/15.96 → 0.82/7.04 | 0.20/12.71 → 0.13/0.67 | 1.76/9.04 → 1.49/1.83 | 47.22/142.00 → 38.33/58.04 | 109.60/1632.96 → 83.40/158.12 | 9.33/69.79 → 4.68/18.21 | 16.32/81.46 → 13.98/53.67 | 186.71/1878.67 → 142.98/223.12 | 180.38/388.54 → 143.04/200.92 |
| 256 / chrome | 0.86/17.50 → 0.81/2.62 | 0.14/1.04 → 0.13/0.83 | 1.56/11.04 → 1.49/2.33 | 11.30/45.62 → 10.54/17.71 | 51.36/135.96 → 47.73/74.17 | 9.93/288.17 → 5.59/149.83 | 20.38/91.92 → 18.63/31.00 | 95.70/386.54 → 85.08/234.62 | 95.62/386.75 → 85.23/234.92 |


### 160x40

| Depth / style | step2 | prepare | reset | fill | draw | dither | diff | wall total | CPU total |
|---|---|---|---|---|---|---|---|---|---|
| truecolor / solid | 1.97/62.67 → 1.23/12.54 | 0.35/33.92 → 0.18/1.33 | 7.06/88.29 → 5.28/16.04 | 49.14/418.00 → 34.55/112.58 | 159.87/2327.75 → 82.54/312.92 | 0.02/0.17 → 0.02/0.12 | 87.80/1747.62 → 66.09/153.62 | 306.44/2525.54 → 190.04/545.62 | 293.25/587.88 → 189.95/510.75 |
| truecolor / outline | 2.73/37.04 → 1.23/4.42 | 0.62/255.50 → 0.18/1.29 | 11.52/3494.00 → 5.21/8.92 | 282.29/12538.12 → 123.86/188.75 | 226.14/25465.54 → 90.21/195.50 | 0.02/2.67 → 0.02/0.04 | 82.63/6988.38 → 38.05/68.17 | 606.23/25696.83 → 258.93/368.38 | 449.19/1051.21 → 258.93/358.83 |
| truecolor / ascii | 2.35/58.12 → 1.16/3.71 | 0.50/157.71 → 0.17/1.12 | 8.49/234.83 → 5.21/10.79 | 62.04/613.33 → 33.73/75.75 | 96.53/6593.29 → 39.72/83.83 | 0.02/0.12 → 0.02/0.04 | 59.28/2691.50 → 35.70/80.46 | 229.44/6683.12 → 115.86/175.46 | 213.87/366.50 → 115.94/151.12 |
| truecolor / braille | 3.22/1192.54 → 1.22/5.46 | 0.39/8.42 → 0.18/1.00 | 7.71/212.71 → 5.18/7.46 | 191.69/1314.42 → 123.18/152.71 | 178.17/1877.79 → 95.91/123.25 | 0.02/0.29 → 0.02/0.08 | 66.16/406.79 → 46.43/66.42 | 447.62/2711.54 → 272.28/325.00 | 435.87/946.21 → 272.33/325.25 |
| truecolor / halftone | 1.86/14.92 → 1.22/7.08 | 0.30/1.71 → 0.18/3.04 | 7.19/61.08 → 5.32/16.50 | 49.03/165.96 → 34.43/144.62 | 70.75/423.04 → 31.34/105.62 | 0.02/0.17 → 0.02/0.08 | 61.56/357.00 → 46.22/889.71 | 190.92/594.88 → 118.89/962.42 | 189.52/366.96 → 118.43/295.54 |
| truecolor / synthwave | 2.00/14.75 → 1.19/4.21 | 0.33/1.62 → 0.18/2.67 | 7.12/206.08 → 5.22/11.21 | 50.11/1926.92 → 33.85/69.58 | 306.48/1409.71 → 167.86/213.79 | 0.02/0.21 → 0.02/0.04 | 88.85/874.04 → 64.01/79.42 | 455.13/2247.58 → 272.49/329.96 | 445.05/940.04 → 272.54/313.29 |
| truecolor / matrix | 1.48/26.12 → 1.16/5.58 | 0.22/3.38 → 0.17/1.12 | 6.27/23.00 → 5.23/15.92 | 22.19/65.88 → 17.90/29.75 | 62.62/294.83 → 51.56/92.25 | 0.02/0.04 → 0.02/0.04 | 43.57/109.50 → 36.40/68.12 | 136.55/389.83 → 112.59/153.88 | 135.99/295.33 → 112.71/136.62 |
| truecolor / topo | 1.52/22.50 → 1.21/4.83 | 0.25/16.62 → 0.19/6.79 | 5.99/23.46 → 5.00/8.29 | 143.35/473.25 → 118.91/169.38 | 339.49/1031.79 → 269.18/352.42 | 0.02/0.08 → 0.02/0.04 | 55.19/116.42 → 45.38/98.92 | 545.99/1381.29 → 440.04/537.92 | 542.87/1020.92 → 439.82/535.58 |
| truecolor / chrome | 1.35/44.08 → 1.15/6.79 | 0.19/1.08 → 0.17/0.92 | 5.85/45.54 → 5.09/14.46 | 38.41/109.25 → 32.83/48.83 | 193.13/430.50 → 165.26/215.62 | 0.02/4.96 → 0.02/0.04 | 76.47/133.67 → 63.67/92.96 | 315.61/657.46 → 268.34/322.21 | 314.36/657.75 → 268.38/303.08 |
| 256 / solid | 1.23/7.21 → 1.15/3.17 | 0.18/1.00 → 0.17/1.08 | 5.47/11.08 → 5.12/11.96 | 35.95/76.58 → 33.06/48.12 | 94.26/154.67 → 73.68/110.33 | 27.79/69.96 → 16.33/62.08 | 67.79/114.17 → 59.66/92.62 | 232.84/297.33 → 189.32/259.29 | 232.47/269.12 → 189.42/259.62 |
| 256 / outline | 1.27/8.08 → 1.25/9.29 | 0.19/0.96 → 0.18/0.92 | 5.35/11.29 → 5.18/30.83 | 128.08/322.71 → 122.65/162.75 | 119.46/180.46 → 90.45/129.71 | 21.79/38.83 → 12.33/53.88 | 40.04/77.79 → 37.99/64.04 | 316.36/559.62 → 270.20/356.88 | 315.98/453.75 → 270.16/338.54 |
| 256 / ascii | 1.15/6.96 → 1.18/4.92 | 0.16/1.00 → 0.17/1.25 | 5.13/15.29 → 5.15/13.46 | 33.67/52.50 → 33.41/89.21 | 44.68/87.38 → 38.53/88.33 | 20.95/51.00 → 12.98/26.46 | 34.69/62.33 → 34.71/70.71 | 140.59/193.75 → 126.29/181.62 | 140.64/171.08 → 126.25/162.38 |
| 256 / braille | 1.25/14.33 → 1.22/4.25 | 0.18/1.12 → 0.18/1.17 | 5.15/10.67 → 5.12/9.25 | 122.71/181.58 → 121.95/191.50 | 112.60/279.25 → 93.74/146.25 | 21.58/57.00 → 13.17/23.12 | 45.93/87.96 → 45.14/90.29 | 309.56/553.38 → 280.69/399.88 | 309.49/517.08 → 280.66/400.12 |
| 256 / halftone | 1.14/3.58 → 1.15/3.25 | 0.17/0.75 → 0.17/0.96 | 5.12/41.42 → 5.11/9.46 | 33.26/70.38 → 32.96/54.83 | 44.49/90.00 → 29.17/71.38 | 21.76/61.29 → 13.14/21.25 | 44.62/91.00 → 43.76/58.25 | 150.72/214.83 → 125.62/184.38 | 150.69/183.00 → 125.73/168.00 |
| 256 / synthwave | 1.16/7.38 → 1.23/3.29 | 0.17/1.17 → 0.17/1.08 | 5.14/16.75 → 5.19/8.83 | 33.56/76.92 → 33.73/87.75 | 192.88/275.75 → 157.13/256.88 | 31.32/61.42 → 21.80/56.50 | 61.61/92.54 → 60.24/124.50 | 325.99/464.79 → 279.65/457.75 | 326.01/465.71 → 279.47/458.08 |
| 256 / matrix | 1.12/2.67 → 1.20/3.29 | 0.16/0.92 → 0.17/0.96 | 5.01/9.04 → 5.16/14.50 | 17.56/68.42 → 17.70/30.83 | 48.53/73.75 → 48.90/74.04 | 20.32/39.92 → 12.87/46.04 | 35.23/51.21 → 35.06/52.29 | 128.09/182.96 → 121.22/154.42 | 128.19/168.71 → 121.35/153.46 |
| 256 / topo | 1.19/11.62 → 1.22/3.88 | 0.18/0.79 → 0.18/1.21 | 4.97/34.54 → 4.98/6.42 | 118.52/163.38 → 118.72/149.12 | 283.53/354.75 → 269.32/345.79 | 24.75/71.08 → 15.51/46.54 | 44.88/66.92 → 45.31/68.38 | 478.17/582.79 → 455.39/566.08 | 477.90/582.92 → 455.22/566.25 |
| 256 / chrome | 1.11/6.62 → 1.16/3.12 | 0.16/1.17 → 0.17/1.00 | 4.93/16.79 → 5.10/23.83 | 32.17/56.50 → 33.20/49.29 | 157.15/272.71 → 158.77/223.88 | 26.88/69.12 → 17.05/62.29 | 58.58/108.83 → 59.40/121.08 | 281.13/483.92 → 275.01/406.04 | 281.06/484.04 → 274.89/399.67 |


### 250x70

| Depth / style | step2 | prepare | reset | fill | draw | dither | diff | wall total | CPU total |
|---|---|---|---|---|---|---|---|---|---|
| truecolor / solid | 1.22/9.12 → 1.25/4.79 | 0.19/1.04 → 0.19/1.17 | 13.99/27.67 → 14.22/25.46 | 89.54/175.08 → 89.87/148.75 | 263.03/414.92 → 208.48/282.54 | 0.02/0.17 → 0.02/0.08 | 172.85/301.17 → 177.08/263.42 | 541.00/843.46 → 491.26/667.83 | 540.76/843.71 → 491.12/668.04 |
| truecolor / outline | 1.27/13.88 → 1.34/7.04 | 0.20/2.08 → 0.20/7.29 | 14.24/178.79 → 14.21/30.92 | 312.11/759.38 → 313.14/405.04 | 299.70/356.92 → 236.90/302.38 | 0.02/0.04 → 0.02/0.08 | 97.16/140.88 → 95.17/176.46 | 724.87/1352.92 → 661.14/787.08 | 724.58/1354.04 → 660.84/770.08 |
| truecolor / ascii | 1.18/17.46 → 1.24/3.67 | 0.18/2.33 → 0.18/1.04 | 13.82/180.46 → 14.25/54.29 | 88.17/596.79 → 90.57/182.96 | 118.85/175.04 → 103.85/155.96 | 0.01/0.08 → 0.02/0.08 | 94.24/167.33 → 95.73/167.29 | 316.61/954.29 → 306.01/411.79 | 316.64/954.50 → 305.81/412.17 |
| truecolor / braille | 1.28/7.88 → 1.32/6.50 | 0.21/2.12 → 0.20/1.33 | 13.97/45.04 → 14.21/38.46 | 310.74/576.58 → 314.09/919.29 | 299.09/436.42 → 259.02/643.25 | 0.02/0.08 → 0.02/0.08 | 120.70/167.79 → 122.30/240.29 | 746.18/1100.62 → 711.33/1573.00 | 745.75/1100.88 → 710.59/1567.58 |
| truecolor / halftone | 1.18/3.54 → 1.21/6.71 | 0.18/1.00 → 0.18/1.38 | 13.83/38.71 → 14.14/44.92 | 88.14/150.04 → 89.39/145.54 | 117.43/162.50 → 77.47/383.42 | 0.02/0.17 → 0.02/0.08 | 119.75/218.12 → 119.20/164.67 | 340.69/512.88 → 301.78/618.21 | 340.53/513.12 → 301.62/349.83 |
| truecolor / synthwave | 1.25/7.04 → 1.47/237.83 | 0.19/1.04 → 0.19/1.17 | 13.89/28.33 → 14.13/22.21 | 89.14/602.96 → 89.58/132.29 | 530.46/818.04 → 447.65/610.12 | 0.02/0.08 → 0.02/0.25 | 169.23/287.12 → 174.31/251.96 | 804.33/1473.62 → 727.52/981.96 | 803.57/1473.79 → 726.69/970.54 |
| truecolor / matrix | 1.10/2.92 → 1.27/4.12 | 0.16/0.88 → 0.18/0.92 | 13.15/40.38 → 14.36/46.25 | 46.04/81.12 → 49.79/85.17 | 119.67/262.38 → 129.70/212.38 | 0.01/0.08 → 0.02/0.08 | 89.30/121.42 → 99.60/172.29 | 269.60/419.38 → 295.09/386.25 | 269.49/346.21 → 294.73/364.71 |
| truecolor / topo | 1.23/18.17 → 1.40/22.21 | 0.19/1.38 → 0.22/3.21 | 13.07/31.62 → 14.13/215.04 | 294.48/919.62 → 308.14/883.50 | 706.55/1898.71 → 700.82/867.46 | 0.01/0.12 → 0.02/0.04 | 113.79/317.79 → 118.39/195.67 | 1129.48/3096.71 → 1143.28/1675.54 | 1128.37/2430.62 → 1142.25/1675.75 |
| truecolor / chrome | 1.09/4.62 → 1.33/28.58 | 0.17/0.92 → 0.20/4.75 | 12.92/25.50 → 14.35/158.12 | 82.61/122.46 → 90.84/611.17 | 421.72/689.25 → 447.00/691.67 | 0.01/0.04 → 0.01/0.08 | 159.30/214.58 → 175.42/417.29 | 677.97/1014.25 → 729.31/1230.50 | 677.96/1011.21 → 727.75/1230.71 |
| 256 / solid | 1.10/3.75 → 1.27/6.08 | 0.17/1.12 → 0.20/1.29 | 13.03/29.54 → 14.21/29.08 | 83.44/159.92 → 90.42/223.71 | 218.31/325.12 → 194.50/408.00 | 62.90/117.62 → 42.87/86.33 | 153.12/342.92 → 165.21/352.50 | 532.22/950.00 → 508.85/869.46 | 532.23/946.92 → 508.20/869.67 |
| 256 / outline | 1.09/2.50 → 1.32/16.67 | 0.17/6.21 → 0.21/2.33 | 12.90/25.67 → 14.12/170.83 | 289.32/428.58 → 311.86/945.33 | 280.24/320.88 → 235.18/336.62 | 49.65/72.12 → 30.66/64.83 | 84.56/122.33 → 95.20/157.79 | 718.08/859.25 → 688.71/1399.50 | 718.22/862.50 → 688.10/1399.71 |
| 256 / ascii | 1.07/5.58 → 1.20/8.46 | 0.16/0.92 → 0.18/1.12 | 12.87/32.38 → 13.84/39.46 | 82.14/348.58 → 87.86/141.33 | 107.58/328.88 → 98.52/159.17 | 50.50/234.71 → 33.15/73.00 | 79.89/170.46 → 89.58/130.62 | 334.36/1115.12 → 324.49/435.29 | 334.49/920.75 → 324.36/435.46 |
| 256 / braille | 1.11/7.83 → 1.30/19.88 | 0.17/0.92 → 0.20/1.04 | 12.93/27.67 → 13.82/25.54 | 289.44/356.00 → 306.90/874.92 | 283.26/358.79 → 252.17/390.50 | 52.42/123.25 → 33.86/78.67 | 110.43/143.00 → 117.10/221.75 | 749.91/874.17 → 725.52/1270.46 | 749.72/824.29 → 724.80/1215.29 |
| 256 / halftone | 1.07/3.00 → 1.29/11.42 | 0.17/11.17 → 0.20/1.25 | 12.88/29.50 → 14.40/52.96 | 82.08/138.12 → 90.92/209.17 | 106.76/154.29 → 77.39/233.17 | 53.13/72.58 → 35.47/82.88 | 106.51/153.54 → 118.76/228.04 | 362.75/491.38 → 338.60/764.58 | 362.97/491.50 → 337.84/758.71 |
| 256 / synthwave | 1.13/3.29 → 1.36/6.42 | 0.18/9.54 → 0.20/0.96 | 13.12/28.04 → 14.28/45.71 | 84.22/178.79 → 90.37/213.79 | 485.74/763.83 → 422.56/1133.67 | 79.04/123.71 → 59.12/103.88 | 151.17/281.00 → 163.42/390.25 | 814.77/1234.62 → 751.49/1604.75 | 814.61/1234.83 → 749.61/1567.42 |
| 256 / matrix | 1.09/6.38 → 1.25/9.92 | 0.16/0.88 → 0.19/1.04 | 12.91/25.42 → 14.11/93.62 | 45.14/79.29 → 48.76/87.25 | 111.96/145.50 → 121.08/191.79 | 50.12/71.79 → 33.22/63.88 | 81.58/114.96 → 91.12/172.25 | 303.11/354.21 → 309.89/402.21 | 303.28/354.42 → 309.39/386.50 |
| 256 / topo | 1.13/3.12 → 1.48/19.62 | 0.18/3.58 → 0.24/3.46 | 12.90/26.42 → 14.17/183.25 | 289.42/354.12 → 308.75/855.29 | 699.13/840.58 → 702.41/1673.71 | 63.42/80.92 → 42.53/214.79 | 108.94/133.62 → 117.73/294.21 | 1175.28/1355.58 → 1187.48/2800.83 | 1175.19/1337.67 → 1184.80/2347.71 |
| 256 / chrome | 1.10/7.38 → 1.23/4.08 | 0.17/1.00 → 0.19/1.25 | 12.93/20.79 → 13.78/24.42 | 82.63/142.88 → 87.34/154.21 | 406.09/786.12 → 416.79/567.83 | 67.85/113.46 → 44.23/75.92 | 152.43/270.46 → 158.28/213.79 | 723.34/1226.62 → 722.00/910.54 | 723.19/986.79 → 721.55/910.79 |


### 300x90

| Depth / style | step2 | prepare | reset | fill | draw | dither | diff | wall total | CPU total |
|---|---|---|---|---|---|---|---|---|---|
| truecolor / solid | 0.83/7.50 → 0.95/12.71 | 0.14/0.92 → 0.16/0.96 | 20.06/36.88 → 21.58/50.12 | 114.25/173.62 → 121.67/187.92 | 368.32/499.75 → 309.90/425.54 | 0.01/0.04 → 0.02/0.04 | 251.60/367.46 → 269.20/382.08 | 755.36/990.38 → 723.64/1008.00 | 755.32/990.58 → 723.29/1008.25 |
| truecolor / outline | 0.95/7.12 → 1.17/27.46 | 0.16/1.25 → 0.20/1.21 | 20.32/65.17 → 22.11/49.50 | 409.89/2158.62 → 440.86/681.83 | 433.80/1670.33 → 360.06/563.08 | 0.01/0.08 → 0.02/0.17 | 137.04/927.17 → 146.86/437.83 | 1002.33/4804.33 → 971.44/1607.42 | 1000.36/2188.08 → 969.79/1320.46 |
| truecolor / ascii | 0.87/11.54 → 0.91/6.12 | 0.14/0.83 → 0.15/1.75 | 20.24/38.17 → 21.61/44.25 | 115.15/198.00 → 121.94/184.79 | 173.11/228.71 → 152.69/213.33 | 0.01/0.04 → 0.02/0.08 | 136.40/285.29 → 143.65/209.67 | 446.09/595.62 → 441.14/581.83 | 446.04/582.71 → 441.03/582.08 |
| truecolor / braille | 0.94/6.33 → 1.10/9.21 | 0.16/1.12 → 0.19/1.46 | 20.30/38.25 → 21.94/61.38 | 410.63/613.00 → 436.20/647.46 | 432.54/971.29 → 390.95/465.96 | 0.02/0.04 → 0.02/0.12 | 176.05/225.75 → 189.82/250.88 | 1040.80/1610.58 → 1040.38/1277.62 | 1040.43/1575.21 → 1039.49/1267.96 |
| truecolor / halftone | 0.90/12.12 → 0.93/7.50 | 0.15/1.00 → 0.16/1.08 | 20.34/84.04 → 21.69/57.25 | 115.26/198.92 → 121.89/193.79 | 164.17/321.79 → 114.63/175.42 | 0.02/0.12 → 0.02/0.04 | 172.60/401.88 → 183.01/288.38 | 473.59/726.21 → 442.49/574.46 | 473.25/713.29 → 442.21/553.38 |
| truecolor / synthwave | 1.01/4.96 → 1.07/7.12 | 0.17/0.83 → 0.18/5.46 | 20.83/62.08 → 22.04/77.92 | 118.75/215.58 → 123.96/671.75 | 782.66/1108.46 → 690.25/2557.67 | 0.02/0.04 → 0.02/0.42 | 256.41/438.38 → 271.35/622.12 | 1180.02/1722.33 → 1109.04/3265.38 | 1178.01/1698.79 → 1106.84/3101.67 |
| truecolor / matrix | 1.15/8.21 → 0.93/3.29 | 0.20/7.50 → 0.15/1.62 | 21.81/105.88 → 21.58/76.67 | 66.99/163.75 → 65.58/110.21 | 190.18/1599.33 → 185.49/299.08 | 0.02/0.08 → 0.02/0.08 | 145.68/292.17 → 145.35/244.67 | 426.20/1965.88 → 419.26/538.29 | 423.90/1103.42 → 418.93/509.12 |
| truecolor / topo | 3.69/442.29 → 1.13/11.92 | 0.61/55.50 → 0.20/1.62 | 32.08/485.58 → 21.45/211.54 | 761.03/48534.67 → 426.40/905.50 | 1534.18/9418.88 → 1009.07/1471.17 | 0.04/4.88 → 0.02/0.08 | 247.48/2403.88 → 171.62/240.33 | 2580.29/51812.75 → 1630.05/2085.42 | 2321.56/4111.08 → 1628.61/2079.96 |
| truecolor / chrome | 0.98/15.38 → 1.14/31.54 | 0.16/1.33 → 0.20/1.08 | 21.09/84.04 → 21.92/71.46 | 118.24/215.71 → 122.96/257.71 | 658.59/923.54 → 667.69/1432.12 | 0.02/0.08 → 0.02/0.12 | 258.77/361.33 → 272.44/355.79 | 1058.01/1456.62 → 1086.53/2009.71 | 1057.31/1456.88 → 1084.49/1994.17 |
| 256 / solid | 1.06/13.83 → 2.49/50.42 | 0.19/9.04 → 0.43/6.12 | 22.12/121.46 → 29.55/236.46 | 125.31/663.67 → 179.28/1703.04 | 363.51/7261.83 → 466.16/3362.58 | 103.41/554.12 → 91.05/1467.62 | 261.35/860.17 → 347.71/2720.75 | 877.13/8453.92 → 1116.97/4114.38 | 866.54/2247.29 → 1071.38/2159.29 |
| 256 / outline | 1.17/11.29 → 3.33/310.92 | 0.21/5.79 → 0.58/18.79 | 22.06/260.71 → 34.29/1589.42 | 441.07/1373.04 → 738.21/3187.83 | 454.49/1156.83 → 514.80/3468.96 | 79.82/257.08 → 69.98/1057.50 | 140.75/422.17 → 202.89/1186.58 | 1139.74/2754.88 → 1564.46/4943.92 | 1136.21/2130.92 → 1503.77/2553.08 |
| 256 / ascii | 1.15/11.54 → 1.10/6.38 | 0.20/6.46 → 0.19/1.46 | 22.36/68.12 → 23.85/184.12 | 126.63/405.54 → 133.40/636.33 | 179.56/571.79 → 162.42/480.17 | 84.71/293.75 → 54.51/164.75 | 139.52/447.17 → 150.73/390.33 | 554.31/1258.29 → 526.38/1509.21 | 552.27/1243.54 → 524.11/1165.79 |
| 256 / braille | 3.70/301.42 → 1.11/3.58 | 0.72/164.79 → 0.19/1.04 | 34.70/716.00 → 22.69/44.21 | 775.95/3999.42 → 449.40/555.42 | 781.94/4651.83 → 401.28/545.92 | 150.86/1007.00 → 53.83/92.58 | 265.43/1370.92 → 187.93/249.25 | 2013.84/7157.46 → 1116.60/1274.67 | 1880.45/3658.17 → 1115.73/1270.96 |
| 256 / halftone | 1.05/11.33 → 0.97/4.29 | 0.18/1.42 → 0.17/8.12 | 22.72/66.33 → 22.12/49.67 | 127.18/339.79 → 124.60/194.42 | 180.26/542.88 → 113.89/211.79 | 90.68/265.08 → 53.15/87.29 | 182.62/383.42 → 182.77/251.92 | 604.87/1477.50 → 497.83/718.92 | 603.81/1385.92 → 497.62/719.21 |
| 256 / synthwave | 1.06/3.92 → 1.04/4.25 | 0.18/1.04 → 0.18/1.38 | 22.18/37.79 → 22.71/54.50 | 125.39/180.46 → 126.88/206.67 | 807.28/1052.71 → 661.47/1547.62 | 131.35/173.00 → 93.41/215.38 | 254.89/813.88 → 262.64/881.21 | 1342.50/1834.75 → 1168.50/2261.29 | 1341.69/1834.96 → 1166.70/2017.38 |
| 256 / matrix | 0.92/3.08 → 0.95/3.75 | 0.15/0.88 → 0.16/1.17 | 21.47/53.96 → 22.11/50.17 | 65.75/94.79 → 67.13/110.42 | 176.39/228.42 → 180.06/238.62 | 81.85/167.50 → 50.87/80.25 | 136.82/196.71 → 143.69/218.25 | 483.52/601.25 → 465.14/558.38 | 483.39/580.62 → 464.81/555.71 |
| 256 / topo | 1.21/7.83 → 2.85/25.96 | 0.22/14.67 → 0.52/11.08 | 21.66/79.50 → 29.38/531.04 | 434.09/782.17 → 633.48/3412.83 | 1087.09/1508.58 → 1312.15/5830.62 | 100.88/262.46 → 87.91/1434.33 | 170.22/225.71 → 221.26/971.96 | 1815.55/2264.88 → 2287.91/7896.21 | 1813.45/2256.17 → 2214.97/4375.50 |
| 256 / chrome | 1.03/16.08 → 2.69/26.71 | 0.17/1.38 → 0.48/5.92 | 21.41/54.92 → 31.10/132.25 | 120.46/193.71 → 186.69/944.96 | 645.17/879.71 → 977.12/3689.75 | 108.78/199.71 → 102.21/785.33 | 252.20/818.21 → 370.61/2373.58 | 1149.40/1806.88 → 1671.22/5022.75 | 1148.24/1807.08 → 1624.40/2982.75 |


## Repeated large-size comparison

The full-sweep 300×90/256 tail above shows a host slowdown across changed **and
unchanged** stages; it alone cannot establish a performance regression. Two
additional baseline/optimized pairs ran in alternating fresh processes, filtered
to 300×90/256. Each still rendered 1,800 frames per style. All nine fingerprints
matched in both pairs. This table takes the lower-CPU-mean run of those two per
build/style, retaining that run's p99/max; it does not pick individual minima
from different stages or discard individual slow frames.

CPU means fell 4.6–15.7% across all styles. Dither means fell 32–43%; matrix's
draw remains column-major and is essentially unchanged. Some absolute maxima
still vary under host contention even when the mean improves.

| Style | CPU mean/p99/max before → after (µs) | Draw mean/max before → after (µs) | Dither mean/max before → after (µs) |
|---|---|---|---|
| solid | 854.05/1102.00/1577.25 → 757.21/848.54/1460.75 | 352.97/725.71 → 292.37/531.00 | 102.62/1113.88 → 64.11/920.08 |
| outline | 1120.02/1268.83/1709.58 → 987.85/1117.62/1283.04 | 449.56/969.71 → 351.14/547.21 | 79.05/406.67 → 45.27/259.83 |
| ascii | 534.79/613.58/829.58 → 469.88/552.50/804.79 | 174.95/271.08 → 145.82/241.29 | 83.20/250.08 → 50.11/143.08 |
| braille | 1178.77/1310.71/1488.25 → 1069.36/1210.79/2225.04 | 456.49/552.25 → 383.88/941.50 | 84.33/134.38 → 50.70/87.67 |
| halftone | 578.18/674.58/899.17 → 487.43/590.25/726.42 | 173.10/268.25 → 112.34/232.67 | 86.67/150.62 → 51.57/118.38 |
| synthwave | 1332.08/1696.96/2086.17 → 1123.33/1292.25/1701.33 | 795.99/1396.25 → 629.80/905.50 | 131.95/811.58 → 89.16/543.54 |
| matrix | 483.81/589.92/1029.88 → 459.18/523.12/703.04 | 175.93/445.71 → 175.72/256.12 | 82.80/561.38 → 49.91/157.71 |
| topo | 1789.78/2012.50/3965.17 → 1674.74/1865.17/2246.12 | 1071.96/2086.54 → 999.24/1160.00 | 99.19/256.96 → 60.74/110.67 |
| chrome | 1143.41/1302.50/1833.08 → 1090.41/1318.17/2285.54 | 638.91/972.83 → 624.77/1614.17 | 108.93/792.96 → 66.35/480.50 |

## Validation and remaining boundaries

- `cargo fmt --check`: passed.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `cargo test`: 374 passed, 11 ignored, zero failures. Render/UI snapshots,
  deterministic simulation tests, clipped/empty areas and offset-buffer sweeps
  passed without changing expected outputs.
- Read-only reviewer found no Critical/Important issues. The benchmark column
  was renamed from `cold_dither_us` to `first_dither_us` as suggested.
- All 72 final dynamic field/cell fingerprints matched. The earlier diagnostic
  sweep also matched all 72. Exhaustive cold pair fingerprints matched across all 262,144
  buckets; the ordinary 8,192-bucket stable-ranking differential test passed.
- Highest total thread CPU across the final full sweep: 0.400 ms at 80×24,
  0.566 ms at 160×40, 2.348 ms at 250×70, 4.376 ms at 300×90. These are observed
  bounds on this machine/workload, not portable worst-case guarantees. The
  repeated 300×90/256 selected runs peaked at 2.286 ms.

Recommendations left for the coordinator: continue the loop/output investigation
in lava-h52.1 and validate presentation in native and embedded Ghostty. A drained
PTY measures the app's CPU/output path but not Ghostty's rendering or GPU cost.
The compute path is well below the 60 fps budget in these runs; wall-clock
preemption under heavy host load remains possible. The current profiles do not
justify a spatial index, new kernel cache, SIMD dependency, altered sim arithmetic,
or precomputing all colour buckets at startup. Do not attribute surviving
presentation hitches to the small merge/split cost without a fresh trace.

Raw samply profiles, symbol sidecars and PTY captures were retained under
`/tmp/lava-real-before*`, `/tmp/lava-overlay-before*`, `/tmp/lava-overlay-after*`
and `/tmp/lava-bench-after*`. Stage logs are `/tmp/lava-final-{before,after}-{1,2}.txt`
and `/tmp/lava-large-{before,after}-{3,4}.txt`. They are temporary local artifacts;
the timings and fingerprints needed for the handoff are recorded above, and the
committed benchmarks reproduce the workloads.
