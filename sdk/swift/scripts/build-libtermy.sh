#!/usr/bin/env bash
# Build libtermy (the termy_core cdylib) and stage it in sdk/swift/lib so the
# Swift package can link and load it.
#
# Usage: scripts/build-libtermy.sh [debug|release]   (default: release)
set -euo pipefail

profile="${1:-release}"
case "$profile" in
  debug) cargo_flags=() ;;
  release) cargo_flags=(--release) ;;
  *) echo "usage: $0 [debug|release]" >&2; exit 64 ;;
esac

package_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "$package_dir/../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
lib_dir="$package_dir/lib"

cargo build ${cargo_flags[@]+"${cargo_flags[@]}"} -p termy_core --lib \
  --manifest-path "$repo_root/Cargo.toml"

case "$(uname -s)" in
  Darwin) library="libtermy_core.dylib" ;;
  Linux) library="libtermy_core.so" ;;
  *) echo "unsupported platform: $(uname -s)" >&2; exit 1 ;;
esac

mkdir -p "$lib_dir"
cp "$target_dir/$profile/$library" "$lib_dir/$library"

if [[ "$library" == *.dylib ]]; then
  # Load through the host's runpath so apps can bundle the dylib in
  # Contents/Frameworks instead of depending on the build directory.
  install_name_tool -id "@rpath/$library" "$lib_dir/$library"
  codesign --force --sign - "$lib_dir/$library" >/dev/null 2>&1 || true
fi

echo "staged $lib_dir/$library ($profile)"
