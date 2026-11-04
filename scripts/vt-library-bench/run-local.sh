#!/usr/bin/env bash
# Build libtermy and libghostty-vt the way the CI workflow does, then run the
# paired comparison on this machine.
#
# Usage: scripts/vt-library-bench/run-local.sh [MiB] [pairs] [--frame]
# Needs cargo, zig 0.16.0 on PATH, a C compiler, python3 and git.
# GHOSTTY_REF selects the Ghostty commit (default: the workflow's pin);
# VT_BENCH_DIR holds the Ghostty checkout and binaries between runs.
set -euo pipefail

mib=32
pairs=6
frame=()
positional=0
for arg in "$@"; do
  case "$arg" in
    --frame) frame=(--frame) ;;
    *)
      if (( positional == 0 )); then mib=$arg; else pairs=$arg; fi
      positional=$((positional + 1))
      ;;
  esac
done

repo=$(git rev-parse --show-toplevel)
ref=${GHOSTTY_REF:-05dcf887ef51ea27821f20979493306ec3a19149}
work=${VT_BENCH_DIR:-$repo/target/vt-library-bench}
ghostty=$work/ghostty
mkdir -p "$work"

if [[ ! -d "$ghostty/.git" ]]; then
  git clone --quiet https://github.com/ghostty-org/ghostty.git "$ghostty"
fi
if [[ "$(git -C "$ghostty" rev-parse HEAD)" != "$(git -C "$ghostty" rev-parse "$ref^{commit}" 2>/dev/null)" ]]; then
  git -C "$ghostty" fetch --quiet origin "$ref"
  git -C "$ghostty" checkout --quiet --detach FETCH_HEAD
  rm -f "$work/bench-ghostty"
fi
if [[ ! -x "$work/bench-ghostty" ]]; then
  (cd "$ghostty" && zig build -Demit-lib-vt -Doptimize=ReleaseFast -Dcpu=baseline)
fi

(cd "$repo" && cargo build --release -p termy_core --lib)

source=$repo/scripts/vt-library-bench/vt_library_bench.c
cc -std=c11 -O2 -Wall -Wextra -Werror -DVT_BACKEND_TERMY \
  -I "$repo/crates/core/include" "$source" \
  -L "$repo/target/release" -ltermy_core -Wl,-rpath,"$repo/target/release" \
  -o "$work/bench-termy"
if [[ ! -x "$work/bench-ghostty" ]]; then
  cc -std=c11 -O2 -Wall -Wextra -Werror -DVT_BACKEND_GHOSTTY \
    -I "$ghostty/include" "$source" \
    -L "$ghostty/zig-out/lib" -lghostty-vt -Wl,-rpath,"$ghostty/zig-out/lib" \
    -o "$work/bench-ghostty"
fi

python3 "$repo/scripts/benchmark-vt-libraries.py" \
  --termy "$work/bench-termy" --ghostty "$work/bench-ghostty" \
  --mib "$mib" --pairs "$pairs" ${frame[@]+"${frame[@]}"} \
  --output "$work/result.json" --markdown "$work/summary.md"
cat "$work/summary.md"
rm "$work/summary.md"
