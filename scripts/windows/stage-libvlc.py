#!/usr/bin/env python3
"""Stage an unmodified official VLC 3.0.24 win64 runtime beside termy.exe.

The zip is the portable archive published by VideoLAN. This copies libvlc,
libvlccore, and the playback plugins. It does not copy vlc.exe, the Qt/skins
UI, locale, lua, or installer files, and it does not modify the source tree.

Provenance (fetched from download.videolan.org, hash checked here):
  https://download.videolan.org/pub/videolan/vlc/3.0.24/win64/vlc-3.0.24-win64.zip
  sha256 fcf30850371ad10c9373cc4f0f4501e7dee49e3e9ae9f20c72fb2661a1ca6323
  published sha256 file: vlc-3.0.24-win64.zip.sha256
  corresponding source: https://download.videolan.org/pub/videolan/vlc/3.0.24/vlc-3.0.24.tar.xz

Do not treat every plugin as LGPL. The archive's COPYING.txt is GPL-2.0.
libvlc itself is LGPL-2.1-or-later; individual plugins differ, including GPL
pieces. Shipping this extract publicly still needs a source offer that
satisfies those licenses. This script only records that obligation.
"""

from __future__ import annotations

import argparse
import hashlib
import shutil
import sys
import zipfile
from pathlib import Path

PINNED_VERSION = "3.0.24"
PINNED_SHA256 = "fcf30850371ad10c9373cc4f0f4501e7dee49e3e9ae9f20c72fb2661a1ca6323"
PINNED_ZIP_URL = (
    "https://download.videolan.org/pub/videolan/vlc/3.0.24/win64/vlc-3.0.24-win64.zip"
)
PINNED_SOURCE_URL = "https://download.videolan.org/pub/videolan/vlc/3.0.24/vlc-3.0.24.tar.xz"
PINNED_SOURCE_SHA256 = "e7cab503d1d7d5849b89d2cf0e1ee60d0ef6d012407791b644b9cfc0cc225fdf"
# SHA512 of ffmpeg-8.1.2.tar.xz recorded in contrib/src/ffmpeg/SHA512SUMS
# inside that VLC source. The bytes of the FFmpeg tarball are not fetched here.
FFMPEG_SHA512 = (
    "b3adc16fe426217bb607da01c4137cee0a9788fc08e077874336db185d2b7287"
    "746a7dc94cf0181ea92cd8afcdb06094ee9456e2986112354f84538cb9a5ed0b"
)

LICENSE_FILES = ("COPYING.txt", "AUTHORS.txt", "THANKS.txt", "NEWS.txt", "README.txt")
# VLC module sources under GPL-2.0-or-later that are not needed to play video.
# libdummy_plugin.dll is also GPL-2.0-or-later (modules/control/dummy.c) and
# stays, because playback passes --intf=dummy. That is a public-release blocker.
GPL_ENCODER_PLUGINS = {
    "libx264_plugin.dll",
    "libx26410b_plugin.dll",
    "libx262_plugin.dll",
    "libx265_plugin.dll",
}
SKIP_TOP = {
    "vlc.exe",
    "vlc-cache-gen.exe",
    "axvlc.dll",
    "vlc.ico",
    "msi",
    "skins",
    "hrtfs",
    "locale",
    "lua",
}
REQUIRED_PLUGINS = (
    "plugins/codec/libavcodec_plugin.dll",
    "plugins/demux/libmp4_plugin.dll",
    "plugins/access/libhttp_plugin.dll",
    "plugins/control/libdummy_plugin.dll",
    "plugins/video_output/libdirect3d11_plugin.dll",
    "plugins/audio_output/libmmdevice_plugin.dll",
)


def fail(message: str) -> None:
    print(f"stage-libvlc: {message}", file=sys.stderr)
    raise SystemExit(1)


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def find_runtime_root(extracted: Path) -> Path:
    if (extracted / "libvlc.dll").is_file():
        return extracted
    found = list(extracted.glob("*/libvlc.dll"))
    if len(found) != 1:
        fail(f"{extracted} does not contain exactly one libvlc.dll")
    return found[0].parent


def dll_says_version(path: Path, version: str) -> bool:
    data = path.read_bytes()
    return version.encode("ascii") in data


def copy_tree(source: Path, dest: Path) -> int:
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    for name in LICENSE_FILES:
        src = source / name
        if not src.is_file():
            fail(f"official tree is missing {name}")
        shutil.copy2(src, dest / name)
    for name in ("libvlc.dll", "libvlccore.dll"):
        src = source / name
        if not src.is_file():
            fail(f"official tree is missing {name}")
        shutil.copy2(src, dest / name)
    plugins = source / "plugins"
    if not plugins.is_dir():
        fail("official tree is missing plugins")
    copied = 0
    for path in plugins.rglob("*"):
        if not path.is_file():
            continue
        relative = path.relative_to(plugins)
        if relative.parts and relative.parts[0] == "gui":
            continue
        if path.name.endswith(".dat") or path.name in GPL_ENCODER_PLUGINS:
            continue
        target = dest / "plugins" / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, target)
        if path.name.endswith("_plugin.dll"):
            copied += 1
    if copied == 0:
        fail("no plugins were copied")
    notice = dest / "TERMY-VLC-PROVENANCE.txt"
    notice.write_text(
        "\n".join(
            [
                "Termy video runtime",
                f"Upstream: VideoLAN VLC {PINNED_VERSION} win64, unmodified extract",
                f"Zip: {PINNED_ZIP_URL}",
                f"Zip sha256: {PINNED_SHA256}",
                f"VLC source archive: {PINNED_SOURCE_URL}",
                f"VLC source sha256: {PINNED_SOURCE_SHA256}",
                "The archive is copied to source/vlc-3.0.24.tar.xz next to these binaries.",
                "COPYING.txt is the GPL-2.0 text shipped in the official win64 zip.",
                "COPYING is the GPL-2.0 text from the VLC source tree. COPYING.LIB is the LGPL-2.1 text from that tree.",
                "libvlc's public header is LGPL-2.1-or-later. modules/codec/avcodec is LGPL-2.1-or-later and links FFmpeg.",
                f"contrib/src/ffmpeg/SHA512SUMS in the VLC source names ffmpeg-8.1.2.tar.xz sha512 {FFMPEG_SHA512}.",
                "That FFmpeg archive is not copied. x264 (modules/codec/x264.c, GPL-2.0-or-later) encoder plugins are omitted.",
                "modules/control/dummy.c is GPL-2.0-or-later and libdummy_plugin.dll is still included because playback requests --intf=dummy.",
                "Not included: vlc.exe, vlc-cache-gen.exe, axvlc.dll, plugins/gui, the x264/x265 encoder plugins, locale, lua, skins, hrtfs, msi.",
                "libvlc.dll imports libvlccore.dll, ADVAPI32.dll, KERNEL32.dll, and msvcrt.dll. Those Windows DLLs are not copied.",
                "This does not show whether termy.exe needs the Visual C++ redistributable.",
                "Not a public-release clearance: Termy stays under its own license, and GPL plugins are still in this folder.",
                "",
            ]
        ),
        encoding="utf-8",
    )
    return copied


def verify_dest(dest: Path) -> None:
    for name in ("libvlc.dll", "libvlccore.dll", *LICENSE_FILES, "TERMY-VLC-PROVENANCE.txt"):
        if not (dest / name).is_file():
            fail(f"staged runtime is missing {name}")
    if not dll_says_version(dest / "libvlc.dll", PINNED_VERSION):
        fail(f"staged libvlc.dll does not contain version {PINNED_VERSION}")
    for relative in REQUIRED_PLUGINS:
        if not (dest / relative).is_file():
            fail(f"staged runtime is missing {relative}")
    if (dest / "vlc.exe").exists() or (dest / "plugins" / "gui").exists():
        fail("staged runtime includes vlc.exe or plugins/gui, which this bundle excludes")
    for name in GPL_ENCODER_PLUGINS:
        if list((dest / "plugins").rglob(name)):
            fail(f"staged runtime still includes GPL encoder {name}")
    source_tar = dest / "source" / "vlc-3.0.24.tar.xz"
    if sha256_file(source_tar) != PINNED_SOURCE_SHA256:
        fail("staged source/vlc-3.0.24.tar.xz is missing or does not match the pinned sha256")
    if not (dest / "COPYING.LIB").is_file() or not (dest / "COPYING").is_file():
        fail("staged runtime is missing COPYING or COPYING.LIB from the VLC source tree")
    count = len(list((dest / "plugins").rglob("*_plugin.dll")))
    if count < 300:
        fail(f"staged plugin count is {count}, expected the official playback set")


def place_source(tar_path: Path, dest: Path) -> None:
    if sha256_file(tar_path) != PINNED_SOURCE_SHA256:
        fail(f"{tar_path} sha256 does not match pinned VLC {PINNED_VERSION} source")
    source_dir = dest / "source"
    source_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(tar_path, source_dir / "vlc-3.0.24.tar.xz")
    import tarfile

    with tarfile.open(tar_path) as archive:
        for member, out_name in (
            ("vlc-3.0.24/COPYING", "COPYING"),
            ("vlc-3.0.24/COPYING.LIB", "COPYING.LIB"),
        ):
            extracted = archive.extractfile(member)
            if extracted is None:
                fail(f"{tar_path} does not contain {member}")
            (dest / out_name).write_bytes(extracted.read())


def stage(source: Path, dest: Path, source_tar: Path) -> None:
    root = find_runtime_root(source)
    if not dll_says_version(root / "libvlc.dll", PINNED_VERSION):
        fail(
            f"{root / 'libvlc.dll'} is not VLC {PINNED_VERSION}. "
            "Refusing to stage an untested build."
        )
    copied = copy_tree(root, dest)
    place_source(source_tar, dest)
    verify_dest(dest)
    print(f"stage-libvlc: {copied} plugins -> {dest}")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--zip", type=Path, help="official vlc-3.0.24-win64.zip")
    source.add_argument(
        "--source-dir",
        type=Path,
        help="extracted official tree or an installed 3.0.24 folder; copied, not modified",
    )
    source.add_argument(
        "--check",
        action="store_true",
        help="only verify an already staged dest",
    )
    parser.add_argument("--dest", type=Path, required=True, help="termy.exe directory / vlc")
    parser.add_argument(
        "--source-tar",
        type=Path,
        help="vlc-3.0.24.tar.xz; downloaded from the pinned URL when omitted",
    )
    args = parser.parse_args()
    if args.check:
        verify_dest(args.dest)
        print(f"stage-libvlc: {args.dest} is complete")
        return
    source_tar = args.source_tar or fetch_source_tar(args.dest)
    if args.zip:
        if not args.zip.is_file():
            fail(f"zip not found: {args.zip}")
        digest = sha256_file(args.zip)
        if digest != PINNED_SHA256:
            fail(f"zip sha256 {digest} != pinned {PINNED_SHA256}")
        extract = dest_extract_parent(args.dest)
        if extract.exists():
            shutil.rmtree(extract)
        extract.mkdir(parents=True)
        with zipfile.ZipFile(args.zip) as archive:
            archive.extractall(extract)
        try:
            stage(extract, args.dest, source_tar)
        finally:
            shutil.rmtree(extract, ignore_errors=True)
        return
    if not args.source_dir.is_dir():
        fail(f"source dir not found: {args.source_dir}")
    print(
        "stage-libvlc: source directory matches the 3.0.24 version string only; "
        "the official zip hash was not checked."
    )
    stage(args.source_dir, args.dest, source_tar)


def fetch_source_tar(dest: Path) -> Path:
    import urllib.request

    target = dest.parent / "vlc-3.0.24.tar.xz.download"
    print(f"stage-libvlc: fetching {PINNED_SOURCE_URL}")
    urllib.request.urlretrieve(PINNED_SOURCE_URL, target)
    if sha256_file(target) != PINNED_SOURCE_SHA256:
        target.unlink(missing_ok=True)
        fail("downloaded VLC source tarball did not match the pinned sha256")
    return target


def dest_extract_parent(dest: Path) -> Path:
    return dest.parent / ".vlc-3.0.24-extract"


if __name__ == "__main__":
    main()
