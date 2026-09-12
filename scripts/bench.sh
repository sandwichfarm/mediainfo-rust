#!/usr/bin/env bash
# Times this crate's `mediainfo` against MediaInfoLib (driven through its C API by tools/oracle,
# i.e. the same work the `mediainfo` CLI does) on the given files. Median of N runs, warm cache.
#   scripts/bench.sh /path/to/libmediainfo.so.0 file...
# Needs: tools/oracle built (cd tools/oracle && cargo build --release), target/release/mediainfo.
set -euo pipefail
lib=$1; shift
here=$(cd "$(dirname "$0")/.." && pwd)
rust="$here/target/release/mediainfo"
oracle="$here/tools/oracle/target/release/oracle"
N=${N:-7}

median_ms() { # cmd...
  local t=()
  for _ in $(seq "$N"); do
    local s=$(date +%s%N); "$@" >/dev/null 2>&1; local e=$(date +%s%N)
    t+=($(( (e - s) / 1000 )))
  done
  printf '%s\n' "${t[@]}" | sort -n | sed -n "$(( (N + 1) / 2 ))p"
}

printf '| File | Size | MediaInfoLib 20.08 | mediainfo-rust | Speed-up |\n|---|---|---|---|---|\n'
for f in "$@"; do
  size=$(du -h "$f" | cut -f1)
  ref=$(median_ms "$oracle" "$lib" inform "$f")
  ours=$(median_ms "$rust" "$f")
  awk -v f="$(basename "$f")" -v s="$size" -v r="$ref" -v o="$ours" 'BEGIN { printf "| %s | %s | %.1f ms | %.1f ms | %.1f× |\n", f, s, r / 1000, o / 1000, (o > 0 ? r / o : 0) }'
done
