# Launch and build

Termy XL is the standalone Trends Desk (`trends_desk` / `trends_desk.exe`). This does not rename Termy, does not edit the Termy shortcut, and does not restart anything listening on port 3489.

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

On Windows, build those commands in a Visual Studio x64 prompt. The desk binary is `trends_desk.exe`. The headless helper, if you copy it, is `termy-xcli.exe`. Do not build or run `x.exe`.

`cargo build -p termy` also builds `termy.exe`. Leave that binary and the existing Termy install where they are. Termy XL does not go in `Program Files\Termy` and is not part of `Termy-*-Setup.exe`.

Install into a separate folder and create `Termy XL.lnk` only:

```powershell
cargo build --release -p termy --bin trends_desk
cargo build --release -p termy_x_cli --bin termy-xcli
powershell -File scripts\windows\install-termy-xl.ps1 -SourceDir target\release
```

For a target-triple build, `-SourceDir` is `target\x86_64-pc-windows-msvc\release`.

The script copies `trends_desk.exe` and, when present, `termy-xcli.exe` into `%LOCALAPPDATA%\Termy XL\`. It writes only:

- `%USERPROFILE%\Desktop\Termy XL.lnk`
- `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Termy XL.lnk`

Both shortcuts launch `trends_desk.exe`. If `Termy XL.lnk` already exists, that shortcut is updated in place. Shortcuts whose target is `termy-7154b8a5.exe`, `Termy.lnk`, and `Termy X.lnk` are not created, renamed, or deleted. The script refuses to use the Termy install directory as the Termy XL folder. Do not point this script at the existing Termy checkout.

The standalone process sets AppUserModelID `lassejlv.TermyXL`. Termy keeps `lassejlv.Termy`.

Termy XL settings, column history, and drafts are stored in `%APPDATA%\Termy XL` (`trends-desk\` and `drafts.json`). That is not `%APPDATA%\termy`. `TERMY_XL_DATA_DIR` overrides the data folder. The program refuses to write inside the Termy X config directory. Copying a draft into Termy X happens only from **Copy draft to Termy X** or `termy-xcli trends-desk handoff <id>`.

## Windows build from a source archive

Use the immutable GitHub archive for the commit SHA. Do not clone or reuse an existing Termy checkout. Rust 1.98.1 and the Visual Studio 2022 x64 environment (`vcvars64.bat`) are required. An RTX 4070 is a normal GPU for this build; Cargo still needs the MSVC environment.

```bat
rustup default 1.98.1
mkdir D:\termyxl
curl -L -o D:\termyxl\termy-xl-src.zip https://github.com/jpanasuk-netizen/termy-x/archive/<FULL_SHA>.zip
tar -xf D:\termyxl\termy-xl-src.zip -C D:\termyxl
cd /d D:\termyxl\termy-x-<FULL_SHA>
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
cargo build --release -p termy --bin trends_desk -p termy_x_cli --bin termy-xcli
powershell -NoProfile -File scripts\windows\install-termy-xl.ps1 -SourceDir target\release
```

If Visual Studio is installed as Build Tools, call `vcvars64.bat` from `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\` instead. The archive includes `Cargo.lock` and the crate sources. It does not include browser sessions, cookies, or API keys. `Termy-*-Setup.exe` from a Termy release is the Termy app, not this window. This branch does not publish a separate installer binary.

## Open

```sh
cargo run -p termy --bin trends_desk
cargo run -p termy --bin trends_desk -- --fixture
termy-xcli trends-desk status
termy-xcli trends-desk run --topic all --focus AI
```

On Windows, Termy XL looks for a visible window titled Termy (not Termy XL) and sits its left edge on that window's right edge, matching its top and height. The 72px dock stays on that edge and the workspace opens to the right. If Termy X is not found, XL uses the right half of the monitor. Elsewhere the same placement is saved, and the live move uses the platform resize. Clicking XL or pressing Ctrl+Shift+X opens at 2560 wide, or the last saved width, clamped only when the space beside Termy X is smaller. Esc collapses it when nothing is selected. One subject fills the page. Each take is its own gold card, and Reply, Edit, and Draft take open a reply box that posts only from Send. From an installed Termy build, `termy --trends-desk` opens that window in the Termy process and does not replace the Termy shortcut. `--fixture` is labeled sample data.

Inside Termy, use **Termy XL** in the command palette or View menu, or **Open Termy XL** on the Trends tab.

A run is successful only when the column says **Fresh**, the report file exists, and the cards contain `https://x.com/.../status/...` links from that call. HTTP 200 from the old browser desk is not this check. Fixture cards are not a successful live run.

## What this environment verified

Commands actually run for this change are recorded in the pull request. A Windows desktop, Chrome, and a logged-in OpenCLI session were not available here, so the shortcut script and a live column were not executed in this session.

GitHub release assets are the existing Termy setup packages. This branch does not publish a Termy XL installer. The Windows worker builds `trends_desk.exe` from this commit and runs `scripts\windows\install-termy-xl.ps1`.
