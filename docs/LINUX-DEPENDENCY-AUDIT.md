# Linux migration — dependency audit (agent L1, branch linux-l1-deps)

Base: `harden/no-bugs-perf-ux` at bc20ab0 (integration of perf branches was
in flight; this worktree branched from that HEAD — rebase before merge if
HEAD moved). Scope: Cargo.toml only. No code, no Windows behaviour change.

## Verdict
The workspace was already mostly correctly gated (winreg/windows-sys,
macOS objc/core-text, Linux zed-font-kit, unix rustix-openpty). The real
cross-platform risks were *implicit* defaults that could drift to native C
backends. Those are now pinned. No dependency was removed or renamed.

## Audit table
| Crate (where) | Class | Why |
|---|---|---|
| anyhow, serde(_json), sha2, uuid, url, base64, unicode-*, compact_str, regex, bincode | shared | pure Rust, no OS surface |
| dirs (workspace) | shared | per-OS dirs internally: %APPDATA% on Windows, XDG on Linux |
| keyring (workspace, core, x_core) | shared | meta-crate, target-gated internally: Windows Credential Manager / macOS Keychain / Linux Secret Service (zbus, crypto-rust) + linux-keyutils (opt-in feature only) |
| ureq (workspace) | shared, **changed** | was `features=["json"]` relying on implicit defaults; now `default-features=false, features=["json","tls","gzip"]` = rustls+webpki-roots. No OpenSSL/native-tls anywhere. Identical feature set to before |
| flume, polling, log, semver, tempfile, fs4, notify, sysinfo, webbrowser, rfd, image, resvg, fontdb, ttf-parser, png | shared | cross-platform; platform backends are internal and already cfg-gated upstream |
| p256, rand_core/getrandom | shared | pure Rust crypto; getrandom picks OS RNG per target |
| gpui-kit =0.7.0 (workspace, default-features=false) | shared | GPUI platform selection is internal to gpui/gpui_platform (X11/Wayland on Linux, Win32 on Windows); do not gate at this level |
| flate2 (core) | shared, **changed** | pinned `default-features=false, features=["rust_backend"]` (miniz_oxide, pure Rust; was implicit default). Blocks zlib-ng/cloudflare C backends |
| rustix, rustix-openpty, libc (core) | unix (existing) | PTY/Unix transport; correct as-is |
| winreg, windows-sys 0.61 (core) | windows (existing) | registry PATH lookup, console ctrl handler, Win32 transport |
| windows 0.61, raw-window-handle, winresource (desktop) | windows (existing) | native_sdk menus/GetCursorPos polling, auto-update ShellExecuteW, mpv child-window host (Win32 HWND), icon build step. Code is `#[cfg(windows)]`/inert stubs elsewhere (x_panel/player.rs, x_panel/mod.rs cursor poll) |
| zed-font-kit (desktop) | linux (existing) | fontconfig-dlopen; correct as-is |
| core-text/core-foundation/cocoa/objc*/dispatch2 (desktop, cli) | macos (existing) | untouched |
| libsqlite3-sys 0.30 (desktop) | not(windows) (existing, untouched) | links system SQLite on Linux → Linux build host needs `libsqlite3-dev` (or a future `bundled` follow-up). sqlx itself: `default-features=false, features=["sqlite","runtime-tokio"]`, no TLS feature → no OpenSSL. Not changed: would alter the working Windows link |
| mimalloc (desktop) | shared, C-backed ⚠️ | libmimalloc-sys builds C via cc. Native Linux/Windows builds fine (measured, perf-a2); *cross* builds need a C compiler for the target. Kept shared because `main.rs` installs it unconditionally; gating it is a code change, out of L1 scope |
| sqlx + tokio(rt only) (desktop) | shared | tokio feature is `rt` only today; Linux io_uring runtime belongs to L2's loop refactor |
| tokio-uring / glommio | **not added** | no cfg(linux) code calls them yet; unused deps add link risk. Designated home: `[target.'cfg(target_os="linux")'.dependencies]` in desktop_app, added in the same commit that first calls them |
| OpenSSL / openssl-sys / native-tls | **absent** ✅ | verified in Cargo.lock: only `openssl-probe` (transitive, via rustls-native-certs in GPUI tree; it probes, it does not link OpenSSL). No direct dep may add it |
| x_core deps (all) | shared | Windows-only X behaviour (CREATE_NO_WINDOW, Chrome/OpenCLI silencer, PATHEXT shims) is hand-rolled user32 FFI under `#[cfg(windows)]` in src/io.rs with a no-op non-Windows stub — no crate to gate |
| cli: clap/ratatui/crossterm/shlex/roxmltree/percent-encoding | shared | crossterm picks Win32 vs Unix backend internally |

## Changes made (Cargo.toml only)
1. `Cargo.toml` (workspace): ureq pinned to explicit pure-Rust TLS set (above).
2. `crates/core/Cargo.toml`: flate2 pinned to pure-Rust `rust_backend`; comment documenting the existing unix/windows split.
3. `crates/desktop_app/Cargo.toml`: comment in the linux section recording the tokio-uring/glommio deferral, the mimalloc cross-compile caveat, and the system-sqlite requirement. No entries moved.
4. `crates/x_core/Cargo.toml`: comment recording why it needs no gated deps.

## Verification
- `cargo check --workspace --target x86_64-unknown-linux-gnu` — see report (baseline at bc20ab0 also passed; must remain passing).
- Windows-target metadata check (`cargo check -p termy_x -p termy_core --target x86_64-pc-windows-msvc`) used to prove no shared dep was broken for Windows; full Windows build/link remains the PC/integration step.
- Known pre-existing gap (not caused here): box lacks system `libxkbcommon-x11`, which affects *linking* one test binary — `cargo check` (no link) is unaffected.

## Follow-ups for other agents
- L2 (cfg loops): add tokio-uring/glommio to desktop_app linux section only with first use; keep Windows fallback.
- Integration: decide `libsqlite3-sys` `bundled` vs documented `libsqlite3-dev` prerequisite; test keyring against a real Linux Secret Service / headless fallback.
- CI (L-pipeline): Linux job needs libsqlite3-dev + GPUI Linux system libs (xkbcommon, wayland/x11) at link/test time.
