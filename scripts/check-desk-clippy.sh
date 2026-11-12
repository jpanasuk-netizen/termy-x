#!/usr/bin/env bash
# Desk sources must be clean under `cargo clippy -- -D warnings` with no allow-list.
# The rest of the workspace still uses the allow-list in architecture-checks.yml.
set -euo pipefail

out="$(mktemp)"
trap 'rm -f "$out"' EXIT

set +e
cargo clippy -p termy_x --lib --message-format=short -- -D warnings >"$out" 2>&1
cargo clippy -p termy --lib --message-format=short -- -D warnings >>"$out" 2>&1
set -e

if rg -n "trends_desk/|desktop_app/src/xl_|desktop_app/src/trends_desk/" "$out"; then
  echo "Desk clippy is not clean under -D warnings." >&2
  exit 1
fi

echo "Desk clippy is clean under -D warnings."
