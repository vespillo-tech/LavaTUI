# A Linux box for checking LavaTUI really works there: stable Rust, a
# D-Bus session bus, a fake MPRIS player and pyte/Pillow for screen
# captures. Used by tools/linux/run.sh; the source is mounted, not copied.
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git pkg-config \
        dbus python3 python3-dbus-next python3-pyte python3-pil \
        fonts-dejavu-core procps \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal \
        --default-toolchain stable --component rustfmt,clippy \
    && rustc --version

# The build lands in a volume (per platform), not in the mounted source.
ENV PYTHONDONTWRITEBYTECODE=1 CARGO_TARGET_DIR=/target LAVATUI_SHOT_FONT=/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf
WORKDIR /src
