# UNVERIFIED — see Cross.toml. Package list mirrors
# .github/workflows/release.yml (build-linux job) / docs/LINUX-BUILD.md.
FROM ghcr.io/cross-rs/x86_64-unknown-linux-gnu:main
RUN apt-get update && apt-get install -y \
  build-essential cmake clang pkg-config \
  libfontconfig-dev libwayland-dev libx11-xcb-dev libxkbcommon-x11-dev \
  libssl-dev libzstd-dev libasound2-dev libvulkan1 \
  && rm -rf /var/lib/apt/lists/*
