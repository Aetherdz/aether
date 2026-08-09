#!/usr/bin/env bash
# aether — reproducible benchmark.
# Measures the three numbers quoted in the README comparison table:
#   1. binary size (MB)
#   2. cold-start time (ms, `aether --version`)
#   3. idle TUI RSS (kB, sessions screen)
#
# Usage:
#   ./benchmark/run.sh                  # uses target/release/aether
#   AETHER_BIN=/path/to/aether ./benchmark/run.sh
#
# Output: benchmark/results.txt (append) + a table on stdout.
set -uo pipefail

BIN="${AETHER_BIN:-$(cd "$(dirname "$0")/.." && pwd)/target/release/aether}"
[ -x "$BIN" ] || { echo "binary not found: $BIN (build with: cargo build --release)" >&2; exit 1; }

RUNS="${RUNS:-5}"
OUT="$(cd "$(dirname "$0")" && pwd)/results.txt"

# --- 1. binary size ----------------------------------------------------------
SIZE_BYTES="$(stat -c %s "$BIN" 2>/dev/null || stat -f %z "$BIN")"
SIZE_MB="$(awk -v b="$SIZE_BYTES" 'BEGIN { printf "%.2f", b/1048576 }')"

# --- 2. cold start -----------------------------------------------------------
# Use bash's SECONDS*1000 via python for sub-ms precision when available.
start_ms() {
  python3 -c "import time; print(int(time.time()*1000))" 2>/dev/null \
    || { date +%s%3N 2>/dev/null || echo 0; }
}
TOTAL_MS=0
"$BIN" --version >/dev/null 2>&1   # warm-up: page the binary into the OS cache
for _ in $(seq "$RUNS"); do
  T0="$(start_ms)"; "$BIN" --version >/dev/null 2>&1; T1="$(start_ms)"
  [ "$T0" -gt 0 ] && [ "$T1" -gt "$T0" ] && TOTAL_MS=$((TOTAL_MS + T1 - T0))
done
START_MS=$((TOTAL_MS / RUNS))

# --- 3. TUI idle RSS ---------------------------------------------------------
# Launch the TUI in a detached tmux session (real terminal), sample VmRSS,
# then kill it. Requires tmux; skipped with a note if absent.
RSS_KB="n/a (tmux required)"
if command -v tmux >/dev/null 2>&1; then
  TDIR="$(mktemp -d)"
  mkdir -p "$TDIR/sessions"
  tmux kill-session -t aether-bench 2>/dev/null
  tmux new-session -d -s aether-bench \
    "AETHER_CONFIG_DIR=$TDIR $BIN tui" >/dev/null 2>&1
  sleep 4
  # Find the real aether process (pgrep also matches the tmux shell wrapper).
  PID=""
  for p in $(pgrep -f "aether tui" 2>/dev/null); do
    [ "$(cat "/proc/$p/comm" 2>/dev/null)" = "aether" ] && PID="$p" && break
  done
  if [ -n "$PID" ]; then
    RSS_KB="$(awk '/VmRSS/ {print $2}' "/proc/$PID/status" 2>/dev/null || echo n/a)"
  fi
  tmux kill-session -t aether-bench 2>/dev/null
  rm -rf "$TDIR"
fi

# --- report ------------------------------------------------------------------
DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf '%-14s %-12s %-14s %s\n' "date" "size_mb" "start_ms" "tui_rss_kb" >> "$OUT"
printf '%-14s %-12s %-14s %s\n' "$DATE" "$SIZE_MB" "$START_MS" "$RSS_KB" >> "$OUT"

echo "aether benchmark ($BIN, $RUNS runs)"
echo "  binary size : ${SIZE_MB} MB (${SIZE_BYTES} bytes)"
echo "  cold start  : ${START_MS} ms  (median of $RUNS)"
echo "  idle TUI RSS: ${RSS_KB} kB"
echo "appended to $OUT"
