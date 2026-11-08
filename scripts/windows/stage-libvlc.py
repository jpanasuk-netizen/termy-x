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
# SHA512 of ffmpeg-8.1.2.tar.xz in contrib/src/ffmpeg/SHA512SUMS of the VLC
# 3.0.24 source. The bytes were checked against the win64 release artifact.
FFMPEG_URL = "https://artifacts.videolan.org/vlc/release-win64/ffmpeg-8.1.2.tar.xz"
FFMPEG_SHA512 = "b3adc16fe426217bb607da01c4137cee0a9788fc08e077874336db185d2b7287746a7dc94cf0181ea92cd8afcdb06094ee9456e2986112354f84538cb9a5ed0b"
# contrib/src/x264/rules.mak pins this git hash. The tarball is the file
# named by the .githash beside the 3.0.24 win64 build. Encoder plugins that
# link it are not copied; the source is still staged because that build used it.
X264_GIT = "e067ab0b530395f90b578f6d05ab0a225e2efdf9"
X264_URL = "https://artifacts.videolan.org/vlc/release-win64/x264-e067ab0b530395f90b578f6d05ab0a225e2efdf9.tar.xz"
X264_SHA256 = "fa11489fecc4655152a7dc3eab1a8b33bf84b67ff6c62f919f07958e1bd44cae"
# Microsoft Visual C++ v14 redistributable 14.51.36247, x64. The file version
# string is inside the executable. Hash matches the official download URL.
VC_REDIST_URL = "https://download.visualstudio.microsoft.com/download/pr/ebdab8e5-1d7b-4d9f-a11b-cbb1720c3b12/843068991DAAA1F73AD9F6239BCE4D0F6A07A51F18C37EA2A867E9BECA71295C/VC_redist.x64.exe"
VC_REDIST_SHA256 = "843068991daaa1f73ad9f6239bce4d0f6a07a51f18c37ea2a867e9beca71295c"
VC_REDIST_VERSION = "14.51.36247"

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


def hash_file(path: Path, algo: str) -> str:
    digest = hashlib.new(algo)
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_file(path: Path) -> str:
    return hash_file(path, "sha256")


def find_runtime_root(extracted: Path) -> Path:
    if (extracted / "libvlc.dll").is_file():
        return extracted
    found = list(extracted.glob("*/libvlc.dll"))
    if len(found) != 1:
        fail(f"{extracted} does not contain exactly one libvlc.dll")
    return found[0].parent


AMD64_MACHINE = 0x8664
ARM64_MACHINE = 0xAA64


def pe_machine(path: Path) -> int | None:
    data = path.read_bytes()[:4096]
    if len(data) < 0x40 or data[:2] != b"MZ":
        return None
    pe_off = int.from_bytes(data[0x3C:0x40], "little")
    if len(data) < pe_off + 6 or data[pe_off : pe_off + 4] != b"PE\0\0":
        return None
    return int.from_bytes(data[pe_off + 4 : pe_off + 6], "little")


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
                f"FFmpeg source: {FFMPEG_URL}",
                f"FFmpeg sha512: {FFMPEG_SHA512}",
                f"x264 git {X264_GIT} source: {X264_URL}",
                f"x264 sha256: {X264_SHA256}",
                "x264 and x265 encoder plugins are omitted. libavcodec_plugin.dll is included. FFmpeg and x264 source archives are copied under source/.",
                "Other contrib archives from the VLC win64 build are not copied, and this script does not rebuild the DLLs.",
                "GPL-2.0-or-later plugins other than the x264/x265 encoders and plugins/gui are still copied. libdummy_plugin.dll is required by --intf=dummy.",
                f"VC++ redistributable {VC_REDIST_VERSION} x64: {VC_REDIST_URL}",
                f"VC++ sha256: {VC_REDIST_SHA256}",
                "That executable is staged in prereqs next to termy.exe. It is not run, and its DLLs are not copied from System32.",
                "modules/control/dummy.c is GPL-2.0-or-later and libdummy_plugin.dll is still included because playback requests --intf=dummy.",
                "Not included: vlc.exe, vlc-cache-gen.exe, axvlc.dll, plugins/gui, the x264/x265 encoder plugins, locale, lua, skins, hrtfs, msi.",
                "libvlc.dll imports libvlccore.dll, ADVAPI32.dll, KERNEL32.dll, and msvcrt.dll. Those Windows DLLs are not copied.",
                "termy.exe additionally imports msvcp140.dll, vcruntime140.dll, and vcruntime140_1.dll. Those come from the VC++ redistributable above, not from a System32 copy.",
                "Not a public-release clearance. Termy stays under its own license. libdummy_plugin.dll is GPL-2.0-or-later.",
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
    libvlc_machine = pe_machine(dest / "libvlc.dll")
    core_machine = pe_machine(dest / "libvlccore.dll")
    if libvlc_machine != AMD64_MACHINE or core_machine != AMD64_MACHINE:
        fail(
            "staged libVLC is not the AMD64 win64 runtime "
            f"(libvlc={libvlc_machine}, core={core_machine}). "
            "The official winarm64 zip is a different archive and is not staged here."
        )
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
    ffmpeg = dest / "source" / "ffmpeg-8.1.2.tar.xz"
    if hash_file(ffmpeg, "sha512") != FFMPEG_SHA512:
        fail("staged source/ffmpeg-8.1.2.tar.xz is missing or does not match VLC's SHA512SUMS")
    x264 = dest / "source" / f"x264-{X264_GIT}.tar.xz"
    if sha256_file(x264) != X264_SHA256:
        fail("staged x264 source archive is missing or does not match the pinned sha256")
    redist = dest.parent / "prereqs" / "VC_redist.x64.exe"
    if sha256_file(redist) != VC_REDIST_SHA256:
        fail("staged prereqs/VC_redist.x64.exe is missing or does not match the pinned sha256")
    if VC_REDIST_VERSION.encode("utf-16le") not in redist.read_bytes():
        fail(f"staged VC_redist.x64.exe does not contain file version {VC_REDIST_VERSION}")
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


def fetch_hashed(url: str, target: Path, algo: str, expect: str) -> Path:
    if target.is_file() and hash_file(target, algo) == expect:
        return target
    import urllib.request

    print(f"stage-libvlc: fetching {url}")
    target.parent.mkdir(parents=True, exist_ok=True)
    urllib.request.urlretrieve(url, target)
    if hash_file(target, algo) != expect:
        target.unlink(missing_ok=True)
        fail(f"downloaded {target.name} did not match the pinned {algo}")
    return target


def place_matching_sources(dest: Path, ffmpeg: Path, x264: Path, redist: Path) -> None:
    if hash_file(ffmpeg, "sha512") != FFMPEG_SHA512:
        fail(f"{ffmpeg} sha512 does not match VLC contrib/src/ffmpeg/SHA512SUMS")
    if sha256_file(x264) != X264_SHA256:
        fail(f"{x264} sha256 does not match the pinned x264 archive")
    if sha256_file(redist) != VC_REDIST_SHA256:
        fail(f"{redist} sha256 does not match the pinned VC++ redistributable")
    source_dir = dest / "source"
    source_dir.mkdir(parents=True, exist_ok=True)
    shutil.copy2(ffmpeg, source_dir / "ffmpeg-8.1.2.tar.xz")
    shutil.copy2(x264, source_dir / f"x264-{X264_GIT}.tar.xz")
    prereq = dest.parent / "prereqs"
    prereq.mkdir(parents=True, exist_ok=True)
    shutil.copy2(redist, prereq / "VC_redist.x64.exe")
    (prereq / "VC_REDIST.txt").write_text(
        "\n".join(
            [
                f"Microsoft Visual C++ v14 Redistributable (x64), file version {VC_REDIST_VERSION}.",
                f"File: VC_redist.x64.exe",
                f"sha256: {VC_REDIST_SHA256}",
                f"URL: {VC_REDIST_URL}",
                "License terms are inside this executable and at",
                "https://learn.microsoft.com/en-us/cpp/windows/redistributing-visual-cpp-files",
                "This is the official installer. It is not a copy of System32 DLLs and it is not",
                "the Visual Studio VC\\Redist merge-module directory. This script does not run it.",
                "termy.exe imports msvcp140.dll, vcruntime140.dll, and vcruntime140_1.dll.",
                "Run VC_redist.x64.exe on a machine that does not already have them.",
                "Supported parameters include /install /passive /norestart. A reboot is not requested here.",
                "",
            ]
        ),
        encoding="utf-8",
    )


def stage(source: Path, dest: Path, source_tar: Path, ffmpeg: Path, x264: Path, redist: Path) -> None:
    root = find_runtime_root(source)
    if not dll_says_version(root / "libvlc.dll", PINNED_VERSION):
        fail(
            f"{root / 'libvlc.dll'} is not VLC {PINNED_VERSION}. "
            "Refusing to stage an untested build."
        )
    copied = copy_tree(root, dest)
    place_source(source_tar, dest)
    place_matching_sources(dest, ffmpeg, x264, redist)
    verify_dest(dest)
    print(f"stage-libvlc: {copied} plugins -> {dest}")


def self_check() -> None:
    import tempfile

    def header(machine: int) -> bytes:
        blob = bytearray(b"MZ" + b"\0" * 0x3C)
        pe_off = 0x40
        blob[0x3C:0x40] = pe_off.to_bytes(4, "little")
        blob.extend(b"\0" * (pe_off - len(blob)))
        blob.extend(b"PE\0\0")
        blob.extend(machine.to_bytes(2, "little"))
        return bytes(blob)

    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        amd = root / "amd.dll"
        arm = root / "arm.dll"
        amd.write_bytes(header(AMD64_MACHINE))
        arm.write_bytes(header(ARM64_MACHINE))
        if pe_machine(amd) != AMD64_MACHINE or pe_machine(arm) != ARM64_MACHINE:
            fail("pe_machine did not read the PE machine field")
        if pe_machine(amd) == pe_machine(arm):
            fail("AMD64 and ARM64 machines must stay distinct")
    print("stage-libvlc: self-check ok")


def main() -> None:
    if len(sys.argv) == 2 and sys.argv[1] == "--self-check":
        self_check()
        return
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
    parser.add_argument("--ffmpeg-tar", type=Path)
    parser.add_argument("--x264-tar", type=Path)
    parser.add_argument("--vc-redist", type=Path)
    args = parser.parse_args()
    if args.check:
        verify_dest(args.dest)
        print(f"stage-libvlc: {args.dest} is complete")
        return
    cache = args.dest.parent
    source_tar = args.source_tar or fetch_source_tar(args.dest)
    ffmpeg = args.ffmpeg_tar or fetch_hashed(
        FFMPEG_URL, cache / "ffmpeg-8.1.2.tar.xz", "sha512", FFMPEG_SHA512
    )
    x264 = args.x264_tar or fetch_hashed(
        X264_URL, cache / f"x264-{X264_GIT}.tar.xz", "sha256", X264_SHA256
    )
    redist = args.vc_redist or fetch_hashed(
        VC_REDIST_URL, cache / "VC_redist.x64.exe", "sha256", VC_REDIST_SHA256
    )
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
            stage(extract, args.dest, source_tar, ffmpeg, x264, redist)
        finally:
            shutil.rmtree(extract, ignore_errors=True)
        return
    if not args.source_dir.is_dir():
        fail(f"source dir not found: {args.source_dir}")
    print(
        "stage-libvlc: source directory matches the 3.0.24 version string only; "
        "the official zip hash was not checked."
    )
    stage(args.source_dir, args.dest, source_tar, ffmpeg, x264, redist)


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
