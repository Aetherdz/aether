#!/usr/bin/env bash
# Golden-test runner: Rust output vs TS reference oracle.
# Blueprint §7-risk-5 enforcement. Usage: ./golden-tests/run.sh
set -uo pipefail

# ---- binary ---------------------------------------------------------------
# CI points here via AETHER_BIN; defaults to the local Rust debug build.
AETHER_BIN="${AETHER_BIN:-/tmp/opencode/aether-target/debug/aether}"
if [ ! -x "$AETHER_BIN" ]; then
  echo "ERROR: aether binary not found or not executable: $AETHER_BIN" >&2
  echo "       Set AETHER_BIN=/path/to/aether (or build the debug binary)." >&2
  exit 1
fi

# ---- phase gate -----------------------------------------------------------
# The fixtures were captured from the OLD full Node/TS CLI (22 commands,
# table output). The Rust CLI currently ships ask/chat/use/models/providers
# (+ sessions/recall in Phase 1) with FLAT output, so full-diff checks would
# fail by design. Active checks are therefore SHAPE-based; anything that
# expects the TS surface (22 commands, box-drawing tables) is phase-gated to
# SKIP, not fail. Fixtures are left untouched.
#
# Format:  name|mode|args...|expect
#   mode=shape  run "$AETHER_BIN" $args and assert on output shape
#   mode=full   strict diff vs fixtures/<name>.golden (for later phases,
#               once fixtures are re-captured from the Rust CLI)
#   mode=skip   phase-gated: never runs, prints SKIP, never fails
PHASE_CHECKS=(
  "aether-help|shape|--help|commands:ask chat use models providers"
  "providers|shape|providers|providers:19"
  "models-zen|shape|models zen|nonempty-flat"
  "sync-status|skip|--|sync is a TS-only command; absent from Rust Phase 0+1 surface"
  "sessions-list-empty|skip|--|fixture is TS table format; Rust sessions output not yet shape-matched"
)

FIX="golden-tests/fixtures"
OUT="golden-tests/out"
mkdir -p "$OUT"

# TS oracle capture used to REGENERATE the fixtures on every run (overwriting
# them). Kept but opt-in so CI never clobbers the goldens.
capture_ts() {
  (cd /home/abdozaik720/aether-cli && node dist/index.js --help      > "/home/abdozaik720/aetherdz/$FIX/aether-help.golden"  2>&1)
  (cd /home/abdozaik720/aether-cli && node dist/index.js providers   > "/home/abdozaik720/aetherdz/$FIX/providers.golden"   2>&1)
  (cd /home/abdozaik720/aether-cli && node dist/index.js models zen  > "/home/abdozaik720/aetherdz/$FIX/models-zen.golden"  2>&1)
  (cd /home/abdozaik720/aether-cli && node dist/index.js sync status > "/home/abdozaik720/aetherdz/$FIX/sync-status.golden" 2>&1)
}
if [ "${REFRESH_FIXTURES:-0}" = "1" ]; then
  echo "=== capture TS oracle (REFRESH_FIXTURES=1) ==="; capture_ts
else
  echo "=== TS oracle capture skipped (set REFRESH_FIXTURES=1 to regenerate fixtures) ==="
fi

# ---- shape assertions -----------------------------------------------------
shape_ok() {
  local out="$1" expect="$2"
  case "$expect" in
    commands:*)
      local missing=0 c
      for c in ${expect#commands:}; do
        grep -qw "$c" <<<"$out" || { echo "  missing command: $c"; missing=1; }
      done
      return "$missing"
      ;;
    providers:*)
      local want="${expect#providers:}" got
      got="$(grep -c '  key: ' <<<"$out")"
      if [ "$got" -eq "$want" ]; then return 0; fi
      echo "  expected $want provider lines, got $got"
      return 1
      ;;
    nonempty-flat)
      [ -n "$out" ] && ! grep -q '[┌│└]' <<<"$out"
      ;;
    *) return 1 ;;
  esac
}

# ---- run phase-gated checks ----------------------------------------------
pass=0; fail=0; skip=0
for entry in "${PHASE_CHECKS[@]}"; do
  IFS='|' read -r name mode args expect <<<"$entry"
  case "$mode" in
    skip)
      echo "SKIP (phase-gated): $name"
      skip=$((skip+1))
      ;;
    full)
      "$AETHER_BIN" $args > "$OUT/$name.out" 2>&1
      if diff -q "$FIX/$name.golden" "$OUT/$name.out" >/dev/null 2>&1; then
        echo "PASS  $name"; pass=$((pass+1))
      else
        echo "FAIL  $name (diff below)"; diff "$FIX/$name.golden" "$OUT/$name.out" | head -12
        fail=$((fail+1))
      fi
      ;;
    shape)
      # shellcheck disable=SC2086  # args intentionally word-split
      out="$("$AETHER_BIN" $args 2>&1)"; rc=$?
      if [ "$rc" -ne 0 ]; then
        echo "FAIL  $name (exit $rc)"; fail=$((fail+1))
      elif shape_ok "$out" "$expect"; then
        echo "PASS  $name"; pass=$((pass+1))
      else
        echo "FAIL  $name (shape: $expect)"; fail=$((fail+1))
      fi
      ;;
    *)
      echo "FAIL  $name (unknown mode '$mode')"; fail=$((fail+1))
      ;;
  esac
done

echo "=== done (pass=$pass fail=$fail skip=$skip) ==="
[ "$fail" -eq 0 ]
