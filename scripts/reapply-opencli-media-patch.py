#!/usr/bin/env python3
"""Re-apply OpenCLI twitter media-attach patch (Hermes local install). Idempotent."""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

MARKER_UTILS = "TERMY-X-MEDIA-PATCH:recoverable-filechooser"
MARKER_POST = "TERMY-X-MEDIA-PATCH:cdp-first-attach"

UTILS_FN = r"""export function isRecoverableFileInputError(error) {
    const msg = error instanceof Error ? error.message : String(error);
    // TERMY-X-MEDIA-PATCH:recoverable-filechooser
    // Extension set-file-input often clicks the input and waits for Page.fileChooserOpened.
    // X's composer file input is frequently hidden/non-interactive, so the chooser never
    // opens within 5s. Treat that as recoverable so CDP/DataTransfer fallbacks can run.
    return /unknown action|not supported|not[-\s]?allowed|notallowederror|filechooseropened|file chooser|set-file-input|setfileinput returned no count/i.test(msg);
}"""

HELPERS = r"""
async function attachImagesViaCdp(page, absPaths) {
    // TERMY-X-MEDIA-PATCH:cdp-first-attach
    if (typeof page.cdp !== 'function') {
        throw new Error('page.cdp not available');
    }
    await page.cdp('DOM.enable', {}).catch(() => undefined);
    const doc = await page.cdp('DOM.getDocument', { depth: 0 });
    const rootNodeId = doc?.root?.nodeId;
    if (typeof rootNodeId !== 'number') {
        throw new Error('DOM.getDocument returned no root node');
    }
    const query = await page.cdp('DOM.querySelector', {
        nodeId: rootNodeId,
        selector: FILE_INPUT_SELECTOR,
    });
    const nodeId = query?.nodeId;
    if (typeof nodeId !== 'number' || nodeId <= 0) {
        throw new Error(`No file input matching ${FILE_INPUT_SELECTOR}`);
    }
    await page.cdp('DOM.setFileInputFiles', { files: absPaths, nodeId });
}

async function attachImages(page, absPaths) {
    // TERMY-X-MEDIA-PATCH:cdp-first-attach
    const errors = [];
    // Prefer CDP: sets files on the hidden input without opening a native chooser.
    try {
        await attachImagesViaCdp(page, absPaths);
        return 'cdp';
    } catch (err) {
        errors.push(`cdp: ${err instanceof Error ? err.message : String(err)}`);
    }
    if (page.setFileInput) {
        try {
            await page.setFileInput(absPaths, FILE_INPUT_SELECTOR);
            return 'setFileInput';
        } catch (err) {
            const msg = err instanceof Error ? err.message : String(err);
            errors.push(`setFileInput: ${msg}`);
            // Always fall through to DataTransfer for chooser timeouts / hidden inputs.
        }
    }
    try {
        await attachImagesViaDataTransfer(page, absPaths);
        return 'datatransfer';
    } catch (err) {
        errors.push(`datatransfer: ${err instanceof Error ? err.message : String(err)}`);
        throw new CommandExecutionError(
            `Image upload failed after CDP/setFileInput/DataTransfer. ${errors.join(' | ')}`
        );
    }
}
"""


def ok(msg: str) -> None:
    print(f"OK: {msg}")


def info(msg: str) -> None:
    print(f"INFO: {msg}")


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def resolve_root(override: str | None) -> Path:
    if override:
        p = Path(override)
        if (p / "clis" / "twitter" / "post.js").is_file():
            return p.resolve()
        fail(f"OpenCliRoot missing clis/twitter/post.js: {override}")

    candidates: list[Path] = []
    local = os.environ.get("LOCALAPPDATA") or os.environ.get("XDG_DATA_HOME")
    home = Path.home()
    if local:
        candidates.append(Path(local) / "hermes" / "node" / "node_modules" / "@jackwener" / "opencli")
    candidates.append(home / "AppData" / "Local" / "hermes" / "node" / "node_modules" / "@jackwener" / "opencli")
    candidates.append(home / ".local" / "share" / "hermes" / "node" / "node_modules" / "@jackwener" / "opencli")

    for c in candidates:
        if (c / "clis" / "twitter" / "post.js").is_file():
            return c.resolve()
    fail("OpenCLI package not found under Hermes. Pass --opencli-root.")


def backup_once(path: Path) -> None:
    bak = Path(str(path) + ".bak-termy-media")
    if not bak.exists():
        bak.write_bytes(path.read_bytes())
        info(f"backup: {bak}")


def patch_utils(utils: Path) -> None:
    raw = utils.read_text(encoding="utf-8")
    if MARKER_UTILS in raw and "filechooseropened" in raw:
        ok(f"utils.js already patched ({MARKER_UTILS})")
        return
    if "filechooseropened" in raw and "isRecoverableFileInputError" in raw:
        if MARKER_UTILS not in raw:
            backup_once(utils)
            nl = "\r\n" if "\r\n" in raw else "\n"
            raw = raw.replace(
                "export function isRecoverableFileInputError(error) {",
                f"export function isRecoverableFileInputError(error) {{{nl}    // {MARKER_UTILS}",
                1,
            )
            utils.write_text(raw, encoding="utf-8", newline="")
            ok("utils.js stamped marker on existing filechooser fix")
            return
        ok("utils.js already patched (filechooser recoverable)")
        return

    backup_once(utils)
    if "export function isRecoverableFileInputError" not in raw:
        fail("utils.js missing isRecoverableFileInputError — opencli layout changed?")
    patched, n = re.subn(
        r"(?ms)export function isRecoverableFileInputError\(error\)\s*\{.*?^\}",
        lambda _m: UTILS_FN.rstrip(),
        raw,
        count=1,
    )
    if n != 1:
        fail("utils.js: failed to rewrite isRecoverableFileInputError")
    utils.write_text(patched, encoding="utf-8", newline="")
    ok(f"utils.js patched ({MARKER_UTILS})")


def patch_post(post: Path) -> None:
    raw = post.read_text(encoding="utf-8")
    nl = "\r\n" if "\r\n" in raw else "\n"

    if "isRecoverableFileInputError" not in raw:
        backup_once(post)
        needle = "from './shared.js';"
        if needle not in raw:
            fail("post.js: cannot find shared.js import to insert utils import")
        raw = raw.replace(
            needle,
            needle + nl + "import { isRecoverableFileInputError } from './utils.js';",
            1,
        )
        ok("post.js: added isRecoverableFileInputError import")

    if "attachImagesViaCdp" not in raw:
        backup_once(post)
        idx = raw.find("cli({")
        if idx < 0:
            fail("post.js: cannot find cli({ insertion point")
        raw = raw[:idx] + HELPERS.rstrip() + nl + nl + raw[idx:]
        ok(f"post.js: inserted attachImagesViaCdp + attachImages ({MARKER_POST})")
    elif MARKER_POST not in raw:
        backup_once(post)
        raw = raw.replace(
            "async function attachImagesViaCdp(page, absPaths) {",
            f"async function attachImagesViaCdp(page, absPaths) {{{nl}    // {MARKER_POST}",
            1,
        )
        ok("post.js: stamped marker on existing CDP helpers")
    else:
        ok(f"post.js helpers already present ({MARKER_POST})")

    if re.search(r"const method = await attachImages\(page, absPaths\)", raw):
        ok("post.js call site already uses attachImages()")
    elif re.search(r"if \(page\.setFileInput\)", raw) and "attachImagesViaDataTransfer(page, absPaths)" in raw:
        backup_once(post)
        alt = (
            r"(?s)(if \(absPaths\.length > 0\) \{\s*await page\.wait\(\{ selector: FILE_INPUT_SELECTOR, timeout: 20 \}\);\s*)"
            r"if \(page\.setFileInput\) \{.*?else \{\s*await attachImagesViaDataTransfer\(page, absPaths\);\s*\}"
        )
        patched, n = re.subn(alt, r"\1const method = await attachImages(page, absPaths);", raw, count=1)
        if n != 1:
            fail("post.js: found stock setFileInput path but regex replace failed — opencli layout changed?")
        patched = patched.replace(
            "Nothing was posted. Retry, or attach a smaller image.",
            "Nothing was posted (attach via ${method}). Retry, or attach a smaller image.",
        )
        raw = patched
        ok("post.js call site switched to attachImages()")
    else:
        info("post.js: no stock setFileInput block (already patched or different layout)")

    post.write_text(raw, encoding="utf-8", newline="")


def verify(utils: Path, post: Path) -> None:
    u = utils.read_text(encoding="utf-8")
    p = post.read_text(encoding="utf-8")
    fails = []
    if "filechooseropened" not in u:
        fails.append("utils.js missing filechooseropened in recoverable regex")
    if "attachImagesViaCdp" not in p:
        fails.append("post.js missing attachImagesViaCdp")
    if "DOM.setFileInputFiles" not in p:
        fails.append("post.js missing DOM.setFileInputFiles")
    if "await attachImages(page, absPaths)" not in p:
        fails.append("post.js missing attachImages() call site")
    if "isRecoverableFileInputError" not in p:
        fails.append("post.js missing isRecoverableFileInputError import/use")
    if fails:
        for f in fails:
            print(f"FAIL: {f}", file=sys.stderr)
        sys.exit(1)
    ok("verify passed (utils.js + post.js media markers)")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--opencli-root", default="", help="Path to @jackwener/opencli package root")
    args = ap.parse_args()
    root = resolve_root(args.opencli_root or None)
    utils = root / "clis" / "twitter" / "utils.js"
    post = root / "clis" / "twitter" / "post.js"
    if not utils.is_file():
        fail(f"missing {utils}")
    if not post.is_file():
        fail(f"missing {post}")
    info(f"OpenCLI root: {root}")
    pkg = root / "package.json"
    if pkg.is_file():
        try:
            ver = json.loads(pkg.read_text(encoding="utf-8")).get("version")
            if ver:
                info(f"version: {ver}")
        except Exception:
            pass
    patch_utils(utils)
    patch_post(post)
    verify(utils, post)
    ok("reapply-opencli-media-patch complete")


if __name__ == "__main__":
    main()
