# Release Packaging

Release packaging is rooted in `scripts/`. GitHub release workflows should call these scripts directly and upload the artifact paths they produce.

## Source of Truth

- macOS DMG: `scripts/build-dmg.sh`
- signed macOS DMG wrapper: `scripts/build-dmg-signed.sh`
- Windows installer: `scripts/build-setup.ps1`
- Windows installer definition: `scripts/installer/termy.iss`
- Linux tarball and AppImage: `scripts/build-linux.sh`
- app icon generation: `scripts/generate-icon.sh`
- release artifact CI: `.github/workflows/release.yml`
- GitHub macOS signing setup: `docs/architecture/macos-release-signing.md`
- stable release finalization and AUR dispatch: `.github/workflows/finalize-stable-release.yml`

The app version used by packaging scripts comes from `crates/desktop_app/Cargo.toml` unless an explicit version is passed.

Pre-release tags such as `v1.0.0-rc.1` are supported. Artifact file names keep the tag as is, while the Linux deb and RPM package versions become `1.0.0~rc.1` and the AUR `pkgver` becomes `1.0.0rc.1`, so package managers order the pre-release before `1.0.0`. Publish pre-releases with GitHub's pre-release flag so stable finalization (latest release and AUR) skips them. The in-app version still comes from `crates/desktop_app/Cargo.toml`.

## Update installation

The macOS updater stages the new app bundle on the destination filesystem before
replacing the installed bundle. Installation failures restore the previous bundle;
if restoration also fails, the error identifies the retained recovery files.

The Linux tarball installer in the updater replaces all three executable entries:
the `termy` launcher, `termy-bin`, and `termy-cli`. It rejects incomplete archives
before changing the installation and stages replacements before publishing them
by rename. Linux in-app update availability remains controlled by the platform gate.

Run the installer regression tests with
`cargo test -p termy --lib auto_update`. On Unix hosts these also exercise
Linux archive installation using temporary directories, without updating the
host's installed application.

## Artifact Paths

- macOS DMG: `dist/Termy-<version>-macos-<arch>[-signed].dmg` (GitHub releases require `-signed`)
- Windows setup: `target/dist/Termy-<version>-windows-<arch>-Setup.exe`
- Linux tarball: `target/dist/Termy-<version>-linux-<arch>.tar.gz`
- Linux AppImage: `target/dist/Termy-<version>-linux-<arch>.AppImage`

Release workflows must upload these exact locations. If a packaging script changes its output path or file name, update this document, `justfile`, `.github/workflows/release.yml`, and `scripts/check-boundaries.sh` in the same change.

## Local Entrypoints

```sh
just build-dmg -- --version 0.3.0 --arch arm64
just build-setup -- -Version 0.3.0 -Arch x64 -Target x86_64-pc-windows-msvc
./scripts/build-linux.sh --version 0.3.0 --arch x86_64 --target x86_64-unknown-linux-gnu
```

Use `scripts/build-dmg-signed.sh` when a Developer ID signing identity is required. Unsigned DMGs should use `scripts/build-dmg.sh` directly.

The release workflow requires the Developer ID certificate and App Store Connect team API key described in [macOS release signing](macos-release-signing.md). Both macOS architectures must be signed and notarized before the release workflow attaches artifacts. Stable release finalization waits for those signed DMGs.

## Windows video runtime

In-panel video loads libVLC from `<install>\vlc` beside `termy.exe`. The git
repo does not contain those binaries. `scripts/build-setup.ps1` refuses to
build the installer unless that folder is complete.

Stage it from the official 3.0.24 win64 zip (tested version, not "latest"):

```powershell
$env:TERMY_VLC_ZIP = "C:\path\vlc-3.0.24-win64.zip"
.\scripts\build-setup.ps1 -Version <version> -Arch x64 -Target x86_64-pc-windows-msvc
```

Zip: `https://download.videolan.org/pub/videolan/vlc/3.0.24/win64/vlc-3.0.24-win64.zip`

sha256: `fcf30850371ad10c9373cc4f0f4501e7dee49e3e9ae9f20c72fb2661a1ca6323`

`TERMY_VLC_SOURCE` may point at an extracted 3.0.24 tree, including
`C:\Program Files\VideoLAN\VLC`. The script copies. It does not install, uninstall,
or edit that tree. A source directory is accepted only when `libvlc.dll` contains
the text `3.0.24`; unlike the zip, its hash is then not checked.

The staged folder contains `libvlc.dll`, `libvlccore.dll`, `plugins\` (without
`plugins\gui`), and `COPYING.txt`, `AUTHORS.txt`, `THANKS.txt`, `NEWS.txt`,
`README.txt`, plus `TERMY-VLC-PROVENANCE.txt`. It does not contain `vlc.exe`.
Playback does not spawn `vlc.exe` or mpv. If `vlc\libvlc.dll` exists beside
Termy, that directory is the engine even when a system VLC is installed. If
that directory exists but is missing a DLL, plugins, or is not 64-bit, playback
stops with an error instead of using the system install.

The same step copies `vlc-3.0.24.tar.xz` to `vlc\source` after checking
`sha256 e7cab503d1d7d5849b89d2cf0e1ee60d0ef6d012407791b644b9cfc0cc225fdf`,
and writes `COPYING` and `COPYING.LIB` from that archive next to the DLLs.
`COPYING.txt` (from the win64 zip) and `COPYING` are GPL-2.0. `COPYING.LIB` is
LGPL-2.1. libVLC's header is LGPL-2.1-or-later. `modules/codec/avcodec` is
LGPL-2.1-or-later and links FFmpeg. `contrib/src/ffmpeg/SHA512SUMS` in the VLC
source names `ffmpeg-8.1.2.tar.xz` with sha512
`b3adc16fe426217bb607da01c4137cee0a9788fc08e077874336db185d2b7287746a7dc94cf0181ea92cd8afcdb06094ee9456e2986112354f84538cb9a5ed0b`.
The bytes at `https://artifacts.videolan.org/vlc/release-win64/ffmpeg-8.1.2.tar.xz`
match that checksum and are copied to `vlc\source`. `contrib/src/x264/rules.mak`
pins git `e067ab0b530395f90b578f6d05ab0a225e2efdf9`. The matching tarball from
the same win64 artifact directory has sha256
`fa11489fecc4655152a7dc3eab1a8b33bf84b67ff6c62f919f07958e1bd44cae` and is
copied too. `modules/codec/x264.c` and
`modules/control/dummy.c` are GPL-2.0-or-later. Encoder plugins `libx264`,
`libx26410b`, and `libx265` are omitted. `libdummy_plugin.dll` remains because
playback passes `--intf=dummy`.

This is not a public-release clearance. Termy is not relicensed. Corresponding
source for FFmpeg and x264 inside the shipped plugins is not in the package.
`termy.exe` imports `msvcp140.dll`, `vcruntime140.dll`, and `vcruntime140_1.dll`
(file version 14.51.36247 on the machine that built `d6238614`). The stage step
copies Microsoft's official `VC_redist.x64.exe` for that version
(sha256 `843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c`)
into `prereqs` beside `termy.exe` and into the installer. It is not executed,
and those DLLs are not taken from System32. This is the redistributable
installer, not an app-local copy of the Visual Studio `VC\Redist` directory.
License terms are inside the executable:
`https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files`.
libVLC's `msvcrt.dll` import is a separate Windows system runtime.

### What else the Windows build already carries

Measured from the official 3.0.24 zip (78.9 MiB compressed, plugins about 140 MiB
uncompressed) and from the desktop build:

| Piece | Needed for video? | How it is provided |
| --- | --- | --- |
| `libvlc.dll`, `libvlccore.dll`, playback plugins | Yes | Staged by `stage-libvlc`. About 140 MiB of plugins before dropping `plugins\gui` (Qt about 17 MiB, skins2 about 2 MiB). |
| `vlc.exe`, Qt/skins UI, locale (about 43 MiB), lua (about 1 MiB), hrtfs, skins | No for in-panel playback | Not staged. |
| `msvcrt.dll`, `kernel32.dll`, and the other imports of those two DLLs | Yes, as Windows system libraries | Not copied. `libvlc.dll` imports `libvlccore.dll`, `ADVAPI32.dll`, `KERNEL32.dll`, and `msvcrt.dll`. |
| Termy MSVC / Universal CRT | Yes for `termy.exe` | The Rust `pc-windows-msvc` toolchain links the CRT dynamically. UCRT is part of Windows 10 and later. Do not copy `vcruntime` out of System32; install the Visual C++ redistributable only if a machine actually lacks it. Not measured on a clean VM here. |
| `Microsoft.UI.Xaml.dll`, `resources.pri`, `termy.manifest` | Yes for the WinUI tab strip, not for video | Already staged by the desktop build and copied by the installer. |
| `conpty.dll`, `OpenConsole.exe` | No | Optional. The installer omits them when absent and uses the system ConPTY. |
| mpv | No on Windows | Windows playback does not call mpv. |
| Fonts, extra certificates, user config, API keys | No | Not bundled. |

## Boundary Rules

- Keep packaging scripts in `scripts/`.
- Keep generated artifacts out of the repo and under `dist/` or `target/dist/`.
- Keep release CI aligned with the script outputs.
- Keep platform-specific installer definitions under `scripts/installer/` unless a platform needs a larger packaging tree.
- File-manager context-menu payloads belong in `scripts/file-manager/` and must be copied by `scripts/build-linux.sh`, `scripts/build-dmg.sh`, and `scripts/installer/termy.iss`. Do not add a parallel `packaging/` tree for those files.

## Validation

Run these checks after packaging or release workflow changes:

```sh
bash -n scripts/build-dmg.sh scripts/build-dmg-signed.sh scripts/setup-macos-signing.sh scripts/build-linux.sh
pwsh -NoProfile -Command '$null = [System.Management.Automation.Language.Parser]::ParseFile("scripts/build-setup.ps1", [ref]$null, [ref]$null)' # when PowerShell is available
just check-boundaries
```

For a local GPUI release performance gate:

```sh
cargo build --release -p termy
./scripts/check-gpui-launch-idle.sh
```

The gate can seed `--plugins`, require `--expect-no-bun`, or load a fixture with
`--workspace-store` to cover plugin and persisted-session regressions.
