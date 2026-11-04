#!/usr/bin/env python3
"""Re-apply OpenCLI twitter write-speed patch (Hermes local install). Idempotent.

Removes only fixed sleeps that sit AFTER the write is already verified,
plus one pre-click pause after composer text verification:

  like.js / unlike.js / retweet.js / unretweet.js:
      `await page.wait(2);` runs after the success state (unlike button,
      unretweet button, ...) has already been observed, right before
      `return [{status: 'success', ...}]`. It delays the result by a
      flat 2s and verifies nothing.
  post.js:
      `await page.wait(1);` between the (already verified) composer
      text insert and submitTweet(). Removing it cannot change what is
      submitted; the text was verified by insertComposerText.

Nothing else is touched: no selector, poll, confirmation, or timeout
logic changes. Confirmation still requires the same observed state /
success toast as upstream. Backups: <file>.bak-termy-speed (once).
Marker: TERMY-X-SPEED-PATCH
"""
from __future__ import annotations

import argparse
import os
import sys
from pathlib import Path

MARKER = "TERMY-X-SPEED-PATCH"
ENGAGE_FILES = ["like.js", "unlike.js", "retweet.js", "unretweet.js"]


def fail(msg: str) -> None:
    print(f"FAIL: {msg}", file=sys.stderr)
    sys.exit(1)


def resolve_root(override: str | None) -> Path:
    if override:
        p = Path(override)
        if (p / "clis" / "twitter" / "like.js").is_file():
            return p.resolve()
        fail(f"OpenCliRoot missing clis/twitter/like.js: {override}")
    candidates: list[Path] = []
    local = os.environ.get("LOCALAPPDATA")
    home = Path.home()
    if local:
        candidates.append(Path(local) / "hermes" / "node" / "node_modules" / "@jackwener" / "opencli")
    candidates.append(home / "AppData" / "Local" / "hermes" / "node" / "node_modules" / "@jackwener" / "opencli")
    for c in candidates:
        if (c / "clis" / "twitter" / "like.js").is_file():
            return c.resolve()
    fail("OpenCLI package not found under Hermes. Pass --opencli-root.")


def backup_once(path: Path) -> None:
    bak = Path(str(path) + ".bak-termy-speed")
    if not bak.exists():
        bak.write_bytes(path.read_bytes())
        print(f"INFO: backup: {bak}")


def patch_engage(path: Path) -> None:
    raw = path.read_text(encoding="utf-8")
    if MARKER in raw:
        print(f"OK: {path.name} already patched ({MARKER})")
        return
    needle = "        await page.wait(2);\n        return [{"
    if needle not in raw:
        fail(f"{path.name}: post-success page.wait(2) block not found - opencli layout changed?")
    backup_once(path)
    raw = raw.replace(
        needle,
        f"        // {MARKER}: no 2s post-success sleep; success was already verified above.\n        return [{{",
        1,
    )
    path.write_text(raw, encoding="utf-8", newline="")
    print(f"OK: {path.name} patched (removed post-success page.wait(2))")


def patch_post(path: Path) -> None:
    raw = path.read_text(encoding="utf-8")
    if MARKER in raw:
        print(f"OK: post.js already patched ({MARKER})")
        return
    needle = "        await page.wait(1);\n        const result = await submitTweet(page, text);"
    if needle not in raw:
        fail("post.js: pre-submit page.wait(1) not found - opencli layout changed?")
    backup_once(path)
    raw = raw.replace(
        needle,
        f"        // {MARKER}: no 1s pause; composer text already verified by insertComposerText.\n        const result = await submitTweet(page, text);",
        1,
    )
    path.write_text(raw, encoding="utf-8", newline="")
    print("OK: post.js patched (removed pre-submit page.wait(1))")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--opencli-root", default="", help="Path to @jackwener/opencli package root")
    args = ap.parse_args()
    root = resolve_root(args.opencli_root or None)
    twitter = root / "clis" / "twitter"
    print(f"INFO: OpenCLI root: {root}")
    for name in ENGAGE_FILES:
        p = twitter / name
        if not p.is_file():
            fail(f"missing {p}")
        patch_engage(p)
    post = twitter / "post.js"
    if not post.is_file():
        fail(f"missing {post}")
    patch_post(post)
    # Verify: markers present, sleeps gone.
    for name in ENGAGE_FILES:
        t = (twitter / name).read_text(encoding="utf-8")
        if MARKER not in t or "await page.wait(2);" in t:
            fail(f"verify failed for {name}")
    t = post.read_text(encoding="utf-8")
    if MARKER not in t or "await page.wait(1);\n        const result = await submitTweet" in t:
        fail("verify failed for post.js")
    print("OK: verify passed (all speed markers present, fixed sleeps gone)")
    print("OK: reapply-opencli-speed-patch complete")


if __name__ == "__main__":
    main()
