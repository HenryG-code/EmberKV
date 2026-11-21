#!/usr/bin/env bash
# Kill the writer at random moments and check every acknowledged record survives.
# Usage: ./kill-loop.sh [runs]     (default 200)
set -euo pipefail                      # stop on any error, unset variable or failed pipe

RUNS="${1:-200}"                       # first argument, or 200 if none given
HERE="$(cd "$(dirname "$0")" && pwd)"  # the folder this script lives in, as a full path
WRITER="$HERE/writer/target/release/writer"
CHECKER="$HERE/checker/target/release/checker"

echo "Building release versions..."
cargo build --release --quiet --manifest-path "$HERE/writer/Cargo.toml"
cargo build --release --quiet --manifest-path "$HERE/checker/Cargo.toml"

# Work on the real disk next to this script, NOT /tmp (which is tmpfs, held in RAM).
mkdir -p "$HERE/crash-runs"
WORK="$(mktemp -d -p "$HERE/crash-runs")"
trap 'rm -rf "$WORK"' EXIT
echo "Testing on: $(df -T "$WORK" | awk 'NR==2 {print $2}') filesystem"

passed=0
failed=0
total_acked=0

for i in $(seq 1 "$RUNS"); do
    rm -f "$WORK/data.log" "$WORK/out.txt"

    # Start the writer in the background. `exec` makes the process ID belong to the writer itself.
    ( cd "$WORK" && exec "$WRITER" 100000000 > out.txt ) &
    pid=$!                             # $! = process ID of the last background job

    printf -v delay "0.%03d" $(( RANDOM % 390 + 10 ))   # 0.010 to 0.399 seconds
    sleep "$delay"

    kill -9 "$pid" 2>/dev/null || true # pull the plug
    wait "$pid" 2>/dev/null || true    # let the shell notice it died

    # Last number the writer PROMISED (0 if it printed nothing yet).
    acked="$(tail -n 1 "$WORK/out.txt" 2>/dev/null || true)"
    acked="${acked:-0}"

    # Last number actually RECOVERED from the file.
    if [[ ! -f "$WORK/data.log" ]]; then
        recovered=0
    elif ! report="$("$CHECKER" "$WORK/data.log" 2>&1)"; then
        echo "run $i: checker refused the file: $report"
        failed=$((failed + 1))
        continue
    else
        recovered="$(grep -o 'Some([0-9]*)' <<< "$report" | grep -o '[0-9]*' || echo 0)"
    fi

    if (( recovered >= acked )); then
        passed=$((passed + 1))
    else
        failed=$((failed + 1))
        echo "run $i: LOST WRITES: acknowledged $acked but recovered only $recovered"
    fi
    total_acked=$((total_acked + acked))
done

echo
echo "runs: $RUNS  passed: $passed  failed: $failed  acknowledged writes checked: $total_acked"
(( failed == 0 ))                      # exit code 0 only if nothing was lost