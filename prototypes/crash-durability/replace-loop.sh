#!/usr/bin/env bash
# Kill a file replacement at random moments and check "data" is always one complete version.
# Usage: ./replace-loop.sh safe|unsafe [runs]
set -euo pipefail

MODE="${1:-safe}"
RUNS="${2:-100}"
HERE="$(cd "$(dirname "$0")" && pwd)"
BIN="$HERE/replace/target/release/replace"

cargo build --release --quiet --manifest-path "$HERE/replace/Cargo.toml"

mkdir -p "$HERE/crash-runs"                       # real disk, not /tmp (lesson 8)
WORK="$(mktemp -d -p "$HERE/crash-runs")"
trap 'rm -rf "$WORK"' EXIT
cd "$WORK"
echo "Testing on: $(df -T . | awk 'NR==2 {print $2}') filesystem, mode: $MODE"

"$BIN" safe 0                                     # start from one complete version

broken=0
for i in $(seq 1 "$RUNS"); do
    "$BIN" "$MODE" "$i" &
    pid=$!
    printf -v delay "0.%03d" $(( RANDOM % 150 ))  # 0 to 149 ms
    sleep "$delay"
    kill -9 "$pid" 2>/dev/null || true
    wait "$pid" 2>/dev/null || true

    # A good file has exactly 200,000 lines, all from the same generation.
    lines=$(wc -l < data 2>/dev/null || echo 0)
    versions=$(sort -u data 2>/dev/null | wc -l)
    if (( lines != 200000 || versions != 1 )); then
        broken=$((broken + 1))
        echo "run $i: BROKEN: $lines lines, $versions different generations"
    fi

    rm -f data.tmp                                # what startup would do with a leftover temp file
done

echo "mode: $MODE  runs: $RUNS  broken files: $broken"
(( broken == 0 ))