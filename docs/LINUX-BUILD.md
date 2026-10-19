# Termy X — Linux build guide (x86_64-unknown-linux-gnu)

Status key: **VERIFIED** = run on this Linux box on 2026-10-19 (America/Chicago)
against commit `bc20ab0`, worktree `termyx-l5` / branch `linux-l5-buildguide`.
**UNVERIFIED** = not run here; do not treat as proven until someone runs it and
updates this file.

## What was actually verified on this Linux box

| Step | Result |
|---|---|
| `rustc --version` / `cargo --version` | rustc/cargo 1.99.1 (stable), host `x86_64-unknown-linux-gnu` **VERIFIED** |
| `rustup target add x86_64-unknown-linux-gnu` | already installed / up to date **VERIFIED** |
| `cargo check -p termy_x -p termy_x_cli -p termy_core -p termy_cli --target x86_64-unknown-linux-gnu` | exit 0 in 41s (1 pre-existing `unused_mut` warning) **VERIFIED** |
| `cargo check -p termy --target x86_64-unknown-linux-gnu` | exit 0 in 1m25s (warnings only) **VERIFIED** |
| `cargo build -p termy -p termy_cli -p termy_x_cli --target x86_64-unknown-linux-gnu` | **FAILED at link for the GUI binaries only**: `rust-lld: error: unable to find library -lxkbcommon-x11` (`termy` and `x_panel_preview`). Missing package is `libxkbcommon-x11-dev` (see below). **VERIFIED failure** |
| `target/x86_64-unknown-linux-gnu/debug/termy-cli --help` | exit 0, prints Termy CLI usage **VERIFIED** |
| `target/x86_64-unknown-linux-gnu/debug/termy-xcli --help` and `... doctor` | exit 0; doctor runs (OpenCLI missing on this box, mock/intent working) **VERIFIED** |
| `bash -n scripts/build-linux.sh scripts/install-linux.sh` and `scripts/build-linux.sh --help` | OK **VERIFIED** |

The CLIs build and run natively on Linux today. The GUI (`termy`) compiles but
does **not link** on a machine without `libxkbcommon-x11-dev`. Nothing was
installed system-wide on the shared box to hide that — the fix is one apt
package (below), to be run by a human on their own machine/WSL.

## Route A — native Linux / WSL2 (recommended)

For: Jeremy's Windows PC using its WSL2 distro `LightBringer`, or any native
Ubuntu/Debian Linux machine (including this box, once a human approves the apt
install). WSL-specific steps are **UNVERIFIED** here (this box is native Linux,
not WSL); the cargo steps after them are the verified ones above.

```powershell
# Windows PowerShell — UNVERIFIED (no WSL on this box)
wsl -d LightBringer
```

```bash
# Inside WSL2 / native Ubuntu — apt block UNVERIFIED on WSL, package list is
# copied verbatim from .github/workflows/release.yml (build-linux job)
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  cmake \
  clang \
  pkg-config \
  libfontconfig-dev \
  libwayland-dev \
  libx11-xcb-dev \
  libxkbcommon-x11-dev \
  libssl-dev \
  libzstd-dev \
  libasound2-dev \
  libvulkan1 \
  rpm

# Rust (skip if rustup already installed — VERIFIED present on this box)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # UNVERIFIED here
rustup target add x86_64-unknown-linux-gnu                        # VERIFIED

# Get the code INSIDE the Linux filesystem (not /mnt/c — slow, and file
# watching/permissions misbehave there) — UNVERIFIED path advice
cd ~ && git clone <this-repo-url> termyx && cd termyx

# Fast proof the non-GUI stack is healthy — VERIFIED (exit 0, 41s)
cargo check -p termy_x -p termy_x_cli -p termy_core -p termy_cli --target x86_64-unknown-linux-gnu

# GUI compile check — VERIFIED (exit 0, 1m25s)
cargo check -p termy --target x86_64-unknown-linux-gnu

# Real binaries — link FAILED on this box only for -lxkbcommon-x11
# (package above fixes it); termy-cli/termy-xcli linked fine — VERIFIED
cargo build -p termy -p termy_cli -p termy_x_cli --target x86_64-unknown-linux-gnu

# Release packaging (existing script; CI release.yml runs exactly this)
./scripts/build-linux.sh --target x86_64-unknown-linux-gnu   # full run UNVERIFIED locally
```

### Run / smoke

```bash
./target/x86_64-unknown-linux-gnu/debug/termy-cli --help   # VERIFIED exit 0
./target/x86_64-unknown-linux-gnu/debug/termy-xcli --help  # VERIFIED exit 0
./target/x86_64-unknown-linux-gnu/debug/termy-xcli doctor  # VERIFIED runs
./target/x86_64-unknown-linux-gnu/debug/termy              # GUI: NOT RUN — see below
```

GUI runtime honesty: `termy` is a GPUI app. It needs a real Wayland session
(`WAYLAND_DISPLAY`) or X11 (`DISPLAY`), plus at runtime `libwayland-client0`,
`libxcb1`, `libxkbcommon0`, `libxkbcommon-x11-0`, `libfreetype6`,
`libfontconfig1`, `libvulkan1` (the same list `scripts/build-linux.sh` puts in
its .deb `Depends:`). The launcher in that script defaults to X11
(`TERMY_LINUX_BACKEND=x11` unsets `WAYLAND_DISPLAY` when `DISPLAY` is set);
set `TERMY_LINUX_BACKEND=wayland` to prefer Wayland. **No GUI launch was
faked or claimed here**: on this box the binary does not exist yet because of
the missing `-dev` package above, and a headless smoke run would not prove
rendering anyway. First real GUI check must be on LightBringer/a desktop:
launch, confirm a window, open the X panel with Ctrl+Shift+X.

WSLg note (**UNVERIFIED**): WSL2 with WSLg provides both `DISPLAY` and
`WAYLAND_DISPLAY`; if the window never appears, try
`TERMY_LINUX_BACKEND=wayland ./termy` and check `echo $DISPLAY $WAYLAND_DISPLAY`.

## Route B — cargo-cross (from Windows or any host with Docker)

**UNVERIFIED end-to-end**: neither `cross` nor Docker/Podman is installed on
this box (`cross: command not found`), so nothing in this route was run.
`Cross.toml` + `Cross.dockerfile` are committed in this branch as a starting
point; the Dockerfile installs the same apt list as Route A on top of the
cross base image.

```bash
cargo install cross --git https://github.com/cross-rs/cross  # UNVERIFIED
cross build --target x86_64-unknown-linux-gnu -p termy_cli -p termy_x_cli  # UNVERIFIED
cross build --target x86_64-unknown-linux-gnu -p termy                     # UNVERIFIED (GUI link)
```

Caveats: cross still needs a running Docker/Podman daemon; on Jeremy's Windows
PC that means Docker Desktop with WSL2 backend (**UNVERIFIED**). Prefer Route A
inside LightBringer — it is simpler and is the route whose cargo steps are
proven. Note cross compiles in a container whose glibc may be older than the
target machine's: a binary built there generally runs on newer distros, not
older ones.

## Toolchain notes

- No `rust-toolchain.toml` is committed, deliberately: CI
  (`.github/workflows/release.yml`, `architecture-checks.yml`) already pins
  `stable` via `dtolnay/rust-toolchain`, and this box builds with stable
  1.99.1. Adding a toolchain file would change behaviour for every existing
  Windows/macOS checkout; if the team wants one, agree it separately.
- Minimum realistic floor: edition 2024 crates, so Rust >= 1.85; use current
  stable. `rustup target list --installed` on this box: 
  `x86_64-unknown-linux-gnu`, `x86_64-pc-windows-msvc` (**VERIFIED**).

## Known gaps / next human steps

1. A human runs the apt block on LightBringer (or approves it on this box),
   re-runs the `cargo build` line, and updates the table above: expected
   outcome is `termy` linking once `libxkbcommon-x11-dev` provides
   `libxkbcommon-x11.so`.
2. First GUI launch smoke (window + X panel) on a real desktop/WSLg.
3. If Route B is wanted, install Docker + cross on one machine, run it once,
   then flip its steps to VERIFIED with the date.
