# Launch and build

This is how to build and open the Trends Desk add-on from this repository. It does not replace a Windows launch script for the separate browser desk, and it does not restart anything listening on port 3489.

## Prerequisites

- Rust 1.98.1 or newer (`rustup default 1.98.1`). The desktop crate uses Cargo edition 2024.
- A C toolchain linker. Linux GPU/UI builds also need `libxkbcommon`, `libxkbcommon-x11`, and `libstdc++`.
- For a live column: OpenCLI 2.x on `PATH` (`opencli`, or `opencli.cmd` on Windows). Auth is the logged-in browser session OpenCLI already uses. This repo does not read cookies or sign you in.
- Optional: `OPENCLI_HERMES_NODE_DIR` set to the directory that contains `node.exe` and `node_modules/@jackwener/opencli/dist/src/main.js`. The desk starts `node` with a relative script path and that directory as the working directory, which is the layout that avoids WSL path rewriting. It still does not use a shell.
- Windows video in the main X panel still needs the VLC runtime described in the repository README. Trends Desk does not play video.
- Do not run `x.exe`. The packaged Termy X CLI is `termy-xcli.exe`.

The official X API stays off. No new paid provider is added, and a failed OpenCLI search does not switch to one.

## Build

From the repository root:

```sh
cargo test -p termy_x --lib trends_desk
cargo build -p termy --bin trends_desk
cargo build -p termy_x_cli --bin termy-xcli
```

Release:

```sh
cargo build --release -p termy --bin trends_desk
cargo build --release -p termy_x_cli --bin termy-xcli
```

On Windows, build the same commands in a Visual Studio x64 prompt, or the environment this repo already uses for `termy.exe`. The desk binary is `trends_desk.exe` next to the rest of the `cargo build` output. Ship it beside `termy.exe` if you want a shortcut. The shortcut target is `trends_desk.exe`, not `x.exe`.

## Open

```sh
cargo run -p termy --bin trends_desk
cargo run -p termy --bin trends_desk -- --fixture
termy-xcli trends-desk status
termy-xcli trends-desk run --topic all --focus AI
```

From an installed Termy build, `termy --trends-desk` opens the window without joining the terminal single-instance handoff. `--fixture` is labeled sample data.

Inside the app, use **X: Trends Desk**, the View menu, or **Open Trends Desk** on the Trends tab.

A run is successful only when the column says **Fresh**, the report file exists, and the cards contain `https://x.com/.../status/...` links from that call. HTTP 200 from the old browser desk is not this check. Fixture cards are not a successful live run.

## What this environment verified

Commands actually run for this change are recorded in the pull request. A Windows desktop, Chrome, and a logged-in OpenCLI session were not available here, so live X results and the Windows package were not executed in this session.
