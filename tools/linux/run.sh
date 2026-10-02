#!/usr/bin/env bash
# Build and test LavaTUI on real Linux, in Docker.
#
#   tools/linux/run.sh                  # native arch: build, tests, MPRIS, pty
#   tools/linux/run.sh --amd64          # linux/amd64 (emulated on Apple silicon)
#   tools/linux/run.sh [--amd64] shell  # a shell in the box, session bus up
#   tools/linux/run.sh [--amd64] <step>...  # build | test | mpris | pty
#
# Steps: build (cargo build --release), test (cargo test, the full suite),
# mpris (the ignored MPRIS integration tests against tools/linux/fake_mpris.py
# on a private session bus), pty (lavatui in a pty with the music widget
# placed, driven by tools/linux/pty_check.py, against the fake as itself and
# as Spotify; screens land in target/linux-check/<arch>/). Cargo's registry and the build dir live in Docker
# volumes per platform, so later runs are incremental.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
platform="linux/$(uname -m | sed 's/x86_64/amd64/; s/aarch64/arm64/')"
if [[ "${1:-}" == "--amd64" ]]; then
    platform=linux/amd64
    shift
fi
tag="lavatui-linux:${platform#linux/}"
vol="lavatui-linux-${platform#linux/}"
steps=("$@")
[[ ${#steps[@]} -eq 0 ]] && steps=(build test mpris pty)

docker build --platform "$platform" -t "$tag" "$here"

tty=()
[[ -t 0 && -t 1 ]] && tty=(-it)
out="$repo/target/linux-check/${platform#linux/}"
mkdir -p "$out"

# Inside: one private session bus for every step (dbus-run-session).
inner='set -euo pipefail
for step in "$@"; do
    echo "=== $step ($(uname -m)) ==="
    case $step in
        build) cargo build --release --locked ;;
        test) cargo test --locked ;;
        mpris) cargo test --locked mpris::live -- --ignored --test-threads=1 --nocapture ;;
        pty) cargo build --release --locked
             python3 tools/linux/pty_check.py --bin /target/release/lavatui --out /out/player
             python3 tools/linux/pty_check.py --bin /target/release/lavatui --out /out/spotify --spotify ;;
        shell) bash ;;
        *) echo "unknown step: $step" >&2; exit 2 ;;
    esac
done'

docker run --rm ${tty[@]+"${tty[@]}"} --platform "$platform" \
    -v "$repo:/src" \
    -v "$out:/out" \
    -v "$vol-cargo:/usr/local/cargo/registry" \
    -v "$vol-target:/target" \
    "$tag" dbus-run-session -- bash -c "$inner" inner "${steps[@]}"
