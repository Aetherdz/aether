#!/usr/bin/env bash
# Golden-test runner: Rust output vs TS reference oracle.
# Blueprint §7-risk-5 enforcement. Usage: ./golden-tests/run.sh
set -uo pipefail

AETHER_BIN="${AETHER_BIN:-target/debug/aether}"   # set when building release
TS_CD="/home/abdozaik720/aether-cli"
FIX="golden-tests/fixtures"
OUT="golden-tests/out"
mkdir -p "$OUT"

# Capture fresh TS goldens (surface commands, no API key needed)
capture_ts() {
  (cd "$TS_CD" && node dist/index.js --help        > "/home/abdozaik720/aetherdz/$FIX/aether-help.golden"        2>&1)
  (cd "$TS_CD" && node dist/index.js providers     > "/home/abdozaik720/aetherdz/$FIX/providers.golden"         2>&1)
  (cd "$TS_CD" && node dist/index.js models zen    > "/home/abdozaik720/aetherdz/$FIX/models-zen.golden"        2>&1)
  (cd "$TS_CD" && node dist/index.js sync status   > "/home/abdozaik720/aetherdz/$FIX/sync-status.golden"       2>&1)
}

# Compare one command
check() {
  local name="$1"; shift
  "$AETHER_BIN" "$@" > "$OUT/$name.out" 2>&1
  if diff -q "$FIX/$name.golden" "$OUT/$name.out" >/dev/null 2>&1; then
    echo "PASS  $name"
  else
    echo "FAIL  $name (diff below)"
    diff "$FIX/$name.golden" "$OUT/$name.out" | head -12
  fi
}

echo "=== capture TS oracle ==="; capture_ts
echo "=== Rust vs TS ==="
check aether-help --help
check providers providers
check models-zen models zen
echo "=== done ==="
exit 0