#!/usr/bin/env bash
# Package libtermy_engine (the engine-only C library) for each target triple:
# dist/libtermy-engine-<target>.tar.gz with the static library, the header, a
# Clang module map and the link flags. On macOS, Apple targets are also
# combined into dist/TermyEngine.xcframework.
#
#   scripts/package-libtermy-engine.sh [target ...]
#
# Targets default to the host. Install them first, e.g.
#   rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
# Cross-compiling to *-linux-musl from another OS needs a musl linker only for
# the shared library; the static library needs none.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
  targets=("$(rustc -vV | sed -n 's/^host: //p')")
fi

dist="$root/dist"
mkdir -p "$dist"
apple_libraries=()

for target in "${targets[@]}"; do
  echo "==> $target"
  cargo build --locked --release -p termy_engine_ffi --lib --target "$target"
  library="target/$target/release/libtermy_engine.a"
  [[ -f "$library" ]] || { echo "missing $library" >&2; exit 1; }

  stage="$dist/libtermy-engine-$target"
  rm -rf "$stage"
  mkdir -p "$stage/lib" "$stage/include"
  cp "$library" "$stage/lib/"
  cp crates/engine_ffi/include/termy_engine.h "$stage/include/"
  cat > "$stage/include/module.modulemap" <<'MODULEMAP'
module CTermyEngine {
  header "termy_engine.h"
  link "termy_engine"
  export *
}
MODULEMAP
  # The system libraries Rust's standard library needs when linked statically.
  cargo rustc --locked --release -p termy_engine_ffi --lib --target "$target" \
    --crate-type staticlib -- --print native-static-libs 2>&1 \
    | sed -n 's/.*native-static-libs: //p' | tail -n 1 > "$stage/lib/link-flags.txt"
  cp crates/engine_ffi/README.md "$stage/"
  cp LICENSE "$stage/"
  tar -C "$dist" -czf "$stage.tar.gz" "$(basename "$stage")"
  echo "    $stage.tar.gz"

  if [[ "$target" == *-apple-darwin ]]; then
    apple_libraries+=("$target")
  fi
done

if [[ ${#apple_libraries[@]} -gt 0 && "$(uname -s)" == "Darwin" ]]; then
  xcframework="$dist/TermyEngine.xcframework"
  rm -rf "$xcframework"
  libraries=()
  if [[ ${#apple_libraries[@]} -gt 1 ]]; then
    # One macOS slice holds every architecture.
    universal="$dist/libtermy-engine-macos-universal"
    mkdir -p "$universal"
    lipo -create -output "$universal/libtermy_engine.a" \
      $(for target in "${apple_libraries[@]}"; do echo "target/$target/release/libtermy_engine.a"; done)
    libraries+=(-library "$universal/libtermy_engine.a")
  else
    libraries+=(-library "target/${apple_libraries[0]}/release/libtermy_engine.a")
  fi
  xcodebuild -create-xcframework "${libraries[@]}" \
    -headers "$dist/libtermy-engine-${apple_libraries[0]}/include" \
    -output "$xcframework"
  echo "    $xcframework"
fi
