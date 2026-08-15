#!/usr/bin/env bash
# aether — maturity benchmark setup.
#
# Clones five real, small, diverse upstream repos (Rust, Python, C, JS,
# HTML/CSS) so the deterministic tool-exercise suite in
# crates/aether-agent/tests/maturity_bench.rs has real code to chew on.
# The test itself never clones — it only reads AETHER_MATURITY_REPOS (or
# /tmp/opencode/maturity-bench) and SKIPs a repo that is absent, so CI
# stays hermetic without network.
#
# Usage:
#   ./benchmark/maturity.sh                    # clone into default dir
#   AETHER_MATURITY_REPOS=/x ./benchmark/maturity.sh
set -uo pipefail

DEST="${AETHER_MATURITY_REPOS:-/tmp/opencode/maturity-bench}"
REPOS=(
  "expressjs/express"
  "psf/requests"
  "BurntSushi/ripgrep"
  "jqlang/jq"
  "twbs/bootstrap"
)

mkdir -p "$DEST"
cd "$DEST"
for repo in "${REPOS[@]}"; do
  name="$(basename "$repo")"
  if [ -d "$name/.git" ]; then
    echo "ok   $name (already present)"
    continue
  fi
  echo "clone $name ..."
  git clone --depth 1 "https://github.com/$repo.git" "$name" || {
    echo "FAIL $name (clone failed; the suite will SKIP it)" >&2
  }
done
echo
echo "repos ready under $DEST — run:"
echo "  AETHER_MATURITY_REPOS=$DEST cargo test -p aether-agent --test maturity_bench"