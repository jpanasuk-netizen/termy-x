# Cross-compilation readiness: native-dependency audit (Linux L4)

Scope: only native / C / TLS dependencies that affect building
`x86_64-unknown-linux-gnu`. No runtime behaviour change. Audited at
`bc20ab0` with `cargo tree --target all` / `--target x86_64-unknown-linux-gnu`
/ `--target x86_64-pc-windows-msvc`.

## TL;DR

* **There was no OpenSSL problem to fix in the selected dependency graph.**
  `cargo tree -i openssl-sys` and `cargo tree -i native-tls` match no package
  on any target, and `Cargo.lock` contains zero `openssl`, `openssl-sys`, or
  `native-tls` packages. The one `openssl-probe` lock entry is only the
  certificate-store prober inside `rustls-native-certs` (pulled by the
  unselected `gpui-pre-reqwest` lock entry; `cargo tree` shows neither is
  selected for linux-gnu or windows-msvc) — it is not linked and does no TLS.
* Our HTTP client `ureq` already defaulted to rustls. It is now **pinned
  explicitly** so that cannot silently change (below).

## Per-crate switches made

| Crate | Before | After | Effect |
|---|---|---|---|
| `ureq` (workspace dep, used by `termy`, `termy_cli`, `termy_core`, `termy_x`) | `version="2", features=["json"]` (implicit defaults `tls`+`gzip`) | `default-features=false, features=["gzip","json","tls"]` | Identical feature set. `tls` in ureq 2.12 = `rustls` (ring provider) + `webpki-roots` + `rustls-pki-types`. No `native-tls`/`openssl-sys` can be unified in. Same endpoints and HTTP behaviour: plain `http://127.0.0.1:11434/v1` (Ollama) / `127.0.0.1:1234` (LM Studio) need no TLS at all; `https://api.x.com`, `https://api.github.com`, theme/plugin registries keep the same webpki-root verification as before. |
| `flate2` (`termy_core`) | `flate2 = "1.1"` (implicit `rust_backend`) | `default-features=false, features=["rust_backend"]` | Identical backend (`miniz_oxide`, pure Rust). The C `zlib`/`zlib-ng` backends can no longer be unified in. |
| `libsqlite3-sys` (direct dep of `termy`, `cfg(not(windows))`) | `libsqlite3-sys = "0.30.1"` | **removed** | Nothing in the code references `libsqlite3_sys`. `sqlx`'s `sqlite` feature already selects `libsqlite3-sys/bundled`, i.e. vendored SQLite compiled with `cc` — no system `libsqlite3`, no `pkg-config` probe for it, no `-lsqlite3`. `Cargo.lock` diff is exactly one removed edge from `termy`. |
| `sqlx` (`termy`) | unchanged: `default-features=false, features=["sqlite","runtime-tokio"]` | unchanged | Already the pure-Rust-driver + bundled-SQLite configuration; `sqlite` (bundled) rather than `sqlite-unbundled` (system lib + bindgen) is the cross-friendly choice. |

## Native inventory for `x86_64-unknown-linux-gnu` (after)

`cargo tree --target x86_64-unknown-linux-gnu --edges normal,build | grep -oE '…-sys …'`

| `-sys` crate | Kind | Cross impact |
|---|---|---|
| `ring` (via `rustls`) | vendored C/asm, built with `cc`, no system lib | none beyond a C compiler for the target |
| `libsqlite3-sys` | vendored SQLite (`bundled` via sqlx), built with `cc` | none; deliberately not moved — replacing it means replacing `workspace_store`'s sqlx/SQLite storage. No pure-Rust SQLite with the same API exists. |
| `libmimalloc-sys` | vendored mimalloc, built with `cc` | none beyond `cc`; deliberately not moved — it is the measured global allocator in `crates/desktop_app/src/main.rs`. Removing it would revert a measured perf change, not fix a link failure. |
| `libbz2-rs-sys` | bzip2 reimplemented in Rust (name says `-sys`, no C) | none |
| `freetype-sys`, `yeslogic-fontconfig-sys` | link system `freetype` (fontconfig is `dlopen` via `zed-font-kit`) | OS package: `libfreetype-dev` on Debian/Ubuntu, provided by cargo-cross images / CI |
| `wayland-sys`, `xkbcommon` (via `xkbcommon` crate, gpui) | link system `wayland`, `xkbcommon`, `xkbcommon-x11`, `xcb` | OS packages: `libwayland-dev libxkbcommon-x11-dev libxcb1-dev` — see "genuinely cannot move" |
| `dirs-sys`, `inotify-sys`, `linux-raw-sys`, `renderdoc-sys`, `wayland-sys` bindings | `libc`/kernel bindings, no extra system libs | none |

Other platforms (not selected on Linux, do not block it): `keyring` uses
`zbus-secret-service-keyring-store` on Linux (pure Rust D-Bus), and its
Apple/Windows stores (`security-framework`, `windows-native-keyring-store`)
are `cfg`-gated inside `keyring` itself. `winreg`/`windows-sys` are
`cfg(windows)`, `core-text`/`cocoa`/`objc2-*` are `cfg(macos)` — none appear
in the linux-gnu tree.

## Genuinely cannot move (with evidence)

The GPUI GUI binary's system GUI libraries. Baseline (pre-change) and
post-change `cargo build --target x86_64-unknown-linux-gnu --workspace` both
fail at exactly the GUI binaries (`termy` and `x_panel_preview`, both gpui) —
with, verbatim:

```
rust-lld: error: unable to find library -lxkbcommon-x11
collect2: error: ld returned 1 exit status
```

Link line also requires `-lxcb -lfreetype -lxkbcommon` (present on this box;
only `-lxkbcommon-x11` is missing: `pkg-config --exists libxkbcommon-x11`
fails, `/usr/lib/x86_64-linux-gnu/libxkbcommon.so` exists but there is no
`libxkbcommon-x11.so`). This comes from `gpui-kit`/`gpui-pre-linux`
(`xkbcommon` with X11 support) and `zed-font-kit`, not from any crate this
project controls, and not from TLS/crypto. Fixing it is an environment
action (`apt install libxkbcommon-x11-dev` / the CI image), not a
Cargo change; no pure-Rust replacement exists for GPUI's X11 keyboard
handling. **This is a pre-existing box gap, not a native-TLS failure.**

## Proof

Baseline at `bc20ab0`, this box:

* `cargo check --target x86_64-unknown-linux-gnu --workspace` — green (1m41s).
* `cargo build --target x86_64-unknown-linux-gnu --workspace` — `termy-cli`
  (192 MB) and `termy-xcli` (108 MB) linked; only the `termy` GUI binary
  failed, on `-lxkbcommon-x11` above.

After this change: same check green, same two CLIs link, same GUI-only
failure (`termy` + `x_panel_preview`, both `-lxkbcommon-x11`), and
`cargo test -p termy_x -p termy_core --lib` passes (740 + 99 tests) — i.e. the build progresses past every TLS/native-dependency step
(rustls/ring, bundled SQLite, mimalloc all compile and link for linux-gnu)
and stops only at the documented missing system GUI library.
